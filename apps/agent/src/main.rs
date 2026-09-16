mod server;
mod service;

use std::{path::PathBuf, sync::Arc};
use tokio::sync::Notify;
use vpn_core::*;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--key-worker") {
        run_key_worker(&args);
        return;
    }

    #[cfg(windows)]
    if args.get(1).map(String::as_str) == Some("--service") {
        service::windows::run();
        return;
    }

    let dev = cfg!(debug_assertions) && args.get(1).map(String::as_str) == Some("--dev");
    let settings = if dev {
        args.get(2)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".runtime/settings.json"))
    } else {
        service::settings_path()
    };
    let shutdown = Arc::new(Notify::new());
    let runtime = tokio::runtime::Runtime::new().expect("runtime initialization");
    let result = runtime.block_on(async {
        let signal = shutdown.clone();
        tokio::spawn(async move { service::wait_for_shutdown(signal).await });
        server::serve(settings, dev, shutdown, || Ok(())).await
    });
    if let Err(error) = result {
        eprintln!("{}: {}", error.code, error.message);
        std::process::exit(1);
    }
}

fn run_key_worker(args: &[String]) {
    let result = (|| -> Result<()> {
        if args.len() != 7 {
            return Err(Error::protocol("invalid key worker arguments"));
        }
        let identity = Identity {
            sid: args[4].clone(),
            net_name: args[5].clone(),
            node_ip: args[6].clone(),
        };
        identifier(&identity.sid)?;
        identifier(&identity.net_name)?;
        identity
            .node_ip
            .parse::<std::net::Ipv4Addr>()
            .map_err(|_| Error::protocol("invalid IP"))?;
        if args[3].len() != 64 || !args[3].bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(Error::protocol("invalid profile"));
        }
        print!(
            "{}",
            keys::ensure_key(&PathBuf::from(&args[2]), &args[3], &identity)?
        );
        Ok(())
    })();
    if result.is_err() {
        std::process::exit(1);
    }
}
