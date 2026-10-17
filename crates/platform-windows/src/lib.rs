#![cfg(windows)]
use std::{ffi::c_void, io, net::Ipv4Addr, ptr};
use windows_sys::Win32::{
    Foundation::*,
    NetworkManagement::{IpHelper::*, Ndis::NET_LUID_LH},
    Networking::WinSock::*,
    Security::{Authorization::*, *},
    System::{Pipes::*, Services::*, Threading::*},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ipv4Route {
    pub interface_index: u32,
    pub prefix_length: u8,
}

pub fn interface_index(alias: &str) -> io::Result<u32> {
    let alias: Vec<u16> = alias.encode_utf16().chain(std::iter::once(0)).collect();
    let mut luid: NET_LUID_LH = unsafe { std::mem::zeroed() };
    let error = unsafe { ConvertInterfaceAliasToLuid(alias.as_ptr(), &mut luid) };
    if error != NO_ERROR {
        return Err(io::Error::from_raw_os_error(error as i32));
    }
    let mut index = 0;
    let error = unsafe { ConvertInterfaceLuidToIndex(&luid, &mut index) };
    if error != NO_ERROR {
        return Err(io::Error::from_raw_os_error(error as i32));
    }
    Ok(index)
}

pub fn best_ipv4_route(target: Ipv4Addr) -> io::Result<Ipv4Route> {
    let octets = target.octets();
    let destination = SOCKADDR_INET {
        Ipv4: SOCKADDR_IN {
            sin_family: AF_INET,
            sin_port: 0,
            sin_addr: IN_ADDR {
                S_un: IN_ADDR_0 {
                    S_un_b: IN_ADDR_0_0 {
                        s_b1: octets[0],
                        s_b2: octets[1],
                        s_b3: octets[2],
                        s_b4: octets[3],
                    },
                },
            },
            sin_zero: [0; 8],
        },
    };
    let mut route: MIB_IPFORWARD_ROW2 = unsafe { std::mem::zeroed() };
    let mut source: SOCKADDR_INET = unsafe { std::mem::zeroed() };
    let error = unsafe {
        GetBestRoute2(
            ptr::null(),
            0,
            ptr::null(),
            &destination,
            0,
            &mut route,
            &mut source,
        )
    };
    if error != NO_ERROR {
        return Err(io::Error::from_raw_os_error(error as i32));
    }
    Ok(Ipv4Route {
        interface_index: route.InterfaceIndex,
        prefix_length: route.DestinationPrefix.PrefixLength,
    })
}

pub fn exact_ipv4_route(
    interface_index: u32,
    target: Ipv4Addr,
    prefix_length: u8,
) -> io::Result<bool> {
    struct Table(*mut MIB_IPFORWARD_TABLE2);
    impl Drop for Table {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { FreeMibTable(self.0.cast()) };
            }
        }
    }

    let mut table = ptr::null_mut();
    let error = unsafe { GetIpForwardTable2(AF_INET, &mut table) };
    if error != NO_ERROR {
        return Err(io::Error::from_raw_os_error(error as i32));
    }
    let table = Table(table);
    let rows = unsafe {
        std::slice::from_raw_parts((*table.0).Table.as_ptr(), (*table.0).NumEntries as usize)
    };
    Ok(rows.iter().any(|row| {
        if row.InterfaceIndex != interface_index
            || row.DestinationPrefix.PrefixLength != prefix_length
        {
            return false;
        }
        let address = unsafe {
            let ipv4 = row.DestinationPrefix.Prefix.Ipv4;
            let bytes = ipv4.sin_addr.S_un.S_un_b;
            Ipv4Addr::new(bytes.s_b1, bytes.s_b2, bytes.s_b3, bytes.s_b4)
        };
        address == target
    }))
}

pub struct PipeSecurity {
    descriptor: PSECURITY_DESCRIPTOR,
}
impl PipeSecurity {
    pub fn new(sid: &str) -> io::Result<Self> {
        if !sid.starts_with("S-1-")
            || !sid
                .bytes()
                .all(|b| b.is_ascii_digit() || b == b'S' || b == b'-')
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid owner SID",
            ));
        }
        let sddl: Vec<u16> = format!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;{sid})\0")
            .encode_utf16()
            .collect();
        let mut descriptor = ptr::null_mut();
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { descriptor })
    }
    pub fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.descriptor,
            bInheritHandle: 0,
        }
    }
}
impl Drop for PipeSecurity {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.descriptor);
        }
    }
}

pub struct ProcessJob(HANDLE);
// A job handle is thread-independent and owned exclusively by this wrapper.
unsafe impl Send for ProcessJob {}
unsafe impl Sync for ProcessJob {}
impl ProcessJob {
    pub fn new() -> io::Result<Self> {
        use windows_sys::Win32::System::JobObjects::*;
        unsafe {
            let job = CreateJobObjectW(ptr::null(), ptr::null());
            if job.is_null() {
                return Err(io::Error::last_os_error());
            }
            let owned = Self(job);
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&info) as u32,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(owned)
        }
    }
    /// # Safety
    /// `process` must be a valid, live process handle for the duration of the call.
    pub unsafe fn assign(&self, process: HANDLE) -> io::Result<()> {
        if unsafe {
            windows_sys::Win32::System::JobObjects::AssignProcessToJobObject(self.0, process)
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}
impl Drop for ProcessJob {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub fn current_sid() -> io::Result<String> {
    unsafe {
        let mut token = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(io::Error::last_os_error());
        }
        let result = token_sid(token);
        CloseHandle(token);
        result
    }
}
unsafe fn token_sid(token: HANDLE) -> io::Result<String> {
    let mut size = 0;
    GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut size);
    let mut data = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
    if GetTokenInformation(token, TokenUser, data.as_mut_ptr().cast(), size, &mut size) == 0 {
        return Err(io::Error::last_os_error());
    }
    let user = &*(data.as_ptr() as *const TOKEN_USER);
    let mut sid_string = ptr::null_mut();
    if ConvertSidToStringSidW(user.User.Sid, &mut sid_string) == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut len = 0;
    while *sid_string.add(len) != 0 {
        len += 1;
    }
    let sid = String::from_utf16_lossy(std::slice::from_raw_parts(sid_string, len));
    LocalFree(sid_string.cast());
    Ok(sid)
}
struct ServiceHandle(SC_HANDLE);
impl Drop for ServiceHandle {
    fn drop(&mut self) {
        unsafe {
            CloseServiceHandle(self.0);
        }
    }
}

fn verify_service_process(pid: u32) -> io::Result<()> {
    unsafe {
        // Querying a LocalSystem process token is normally denied to the interactive user.
        // The SCM exposes the registered service PID with SERVICE_QUERY_STATUS instead; an
        // unprivileged process cannot register or replace that service to impersonate the agent.
        let manager = ServiceHandle(OpenSCManagerW(ptr::null(), ptr::null(), SC_MANAGER_CONNECT));
        if manager.0.is_null() {
            return Err(io::Error::last_os_error());
        }
        let service_name: Vec<u16> = "TincClientAgent\0".encode_utf16().collect();
        let service = ServiceHandle(OpenServiceW(
            manager.0,
            service_name.as_ptr(),
            SERVICE_QUERY_STATUS,
        ));
        if service.0.is_null() {
            return Err(io::Error::last_os_error());
        }
        let mut status: SERVICE_STATUS_PROCESS = std::mem::zeroed();
        let mut needed = 0;
        if QueryServiceStatusEx(
            service.0,
            SC_STATUS_PROCESS_INFO,
            (&mut status as *mut SERVICE_STATUS_PROCESS).cast(),
            std::mem::size_of::<SERVICE_STATUS_PROCESS>() as u32,
            &mut needed,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        if status.dwCurrentState != SERVICE_RUNNING || status.dwProcessId != pid {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "named pipe server is not the running TincClientAgent service",
            ));
        }
        Ok(())
    }
}

// Clients authenticate the server, preventing another local process from impersonating it.
/// # Safety
/// `pipe` must remain a valid connected named-pipe client handle during the call.
pub unsafe fn verify_server(pipe: *mut c_void, dev: bool) -> io::Result<()> {
    unsafe {
        let mut pid = 0;
        if GetNamedPipeServerProcessId(pipe, &mut pid) == 0 {
            return Err(io::Error::last_os_error());
        }
        if !dev {
            return verify_service_process(pid);
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return Err(io::Error::last_os_error());
        }
        let mut token = ptr::null_mut();
        let opened = OpenProcessToken(process, TOKEN_QUERY, &mut token);
        CloseHandle(process);
        if opened == 0 {
            return Err(io::Error::last_os_error());
        }
        let sid = token_sid(token);
        CloseHandle(token);
        if sid? != current_sid()? {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "untrusted pipe server",
            ));
        }
        Ok(())
    }
}
