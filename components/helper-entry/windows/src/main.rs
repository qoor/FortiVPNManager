//! The Windows Helper service's executable, which the service control manager starts.

#[cfg(windows)]
fn main() {
    if let Err(error) = fvm_helper_entry_windows::run() {
        eprintln!("fvm-helper runs only as the FortiVPNManager Helper service: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("fvm-helper is the Windows Helper service");
    std::process::exit(1);
}
