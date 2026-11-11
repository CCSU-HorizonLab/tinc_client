# TincLink Android Phase 0

This is an isolated native Android client prototype. It does not replace the Windows/Linux
desktop clients and does not yet contain a Tinc protocol engine.

## What Phase 0 proves

- The app asks Android for VPN consent through `VpnService.prepare`.
- A foreground `VpnService` creates a non-blocking TUN file descriptor.
- Only `192.0.2.1/32` (a documentation-only address) is routed to the Phase 0 TUN.
- The descriptor is retained by the service and closed on user stop, service teardown, or
  system revocation.
- The UI and notification explicitly distinguish "TUN ready" from "Tinc connected".

Phase 0 does **not** prove gateway reachability, Tinc authentication, packet transport, or
Android-to-Windows/Linux connectivity.

## Required local toolchain

- JDK 17 or newer (the repository machine currently has JDK 21)
- Android Studio with Android SDK Platform 36
- Android SDK Build-Tools 36.x and Platform-Tools (`adb`)
- Gradle 8.13 (required by Android Gradle Plugin 8.13.2)

The Gradle wrapper binary is intentionally not generated on a machine without a trusted Gradle
installation. After installing the toolchain, run this once from this directory:

```text
gradle wrapper --gradle-version 8.13
```

## Build and test

```text
./gradlew testDebugUnitTest
./gradlew lintDebug
./gradlew assembleDebug
```

Expected APK:

```text
app/build/outputs/apk/debug/app-debug.apk
```

## Real-device Phase 0 acceptance

1. Enable USB debugging and confirm the test device is visible with `adb devices`.
2. Install with `adb install -r app/build/outputs/apk/debug/app-debug.apk`.
3. Open TincLink Android and press **申请权限并创建 TUN**.
4. Accept the Android system VPN dialog.
5. Confirm the UI says `TUN_READY`, the foreground notification is visible, and Android shows
   the VPN indicator.
6. Press **关闭并释放 TUN** and confirm the indicator and notification disappear.
7. Collect evidence without credentials:

```text
adb shell dumpsys vpn
adb logcat -d -s AndroidRuntime ActivityManager Vpn
```

Do not reuse a Windows/Linux node ID, virtual IP, private key, or profile for this test.

## Next integration boundary

The future Tinc engine must accept the `VpnService` TUN descriptor without root, protect its
underlying transport sockets from VPN recursion, and close or return descriptor ownership on
every failure path. The current server uses Tinc 1.0.x-compatible router configuration, while
rootless Android file-descriptor support is a Tinc 1.1 capability. Protocol, configuration,
licensing, and real gateway compatibility must be validated before bundling any engine.
