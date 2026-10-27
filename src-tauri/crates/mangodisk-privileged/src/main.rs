//! Machine-scoped capability host. The desktop application remains unprivileged.

#[cfg(any(windows, test))]
mod vendor_payloads;

fn main() {
    #[cfg(windows)]
    if let Err(error) = mangodisk_privileged::windows::run(
        vendor_payloads::PAWNIO_INSTALLER_BYTES,
        vendor_payloads::INTEL_MSR_MODULE_BYTES,
        vendor_payloads::AMD_ZEN_MODULE_BYTES,
    ) {
        let code = mangodisk_privileged::signature_verification::native_code(error.as_ref())
            .map(|code| code as i32)
            .unwrap_or_else(|| {
                error
                    .downcast_ref::<std::io::Error>()
                    .and_then(|error| error.raw_os_error())
                    .filter(|code| matches!(*code, 3010 | 1618 | 1603))
                    .unwrap_or_else(|| {
                        if error
                            .downcast_ref::<std::io::Error>()
                            .is_some_and(|error| error.kind() == std::io::ErrorKind::InvalidData)
                        {
                            mangodisk_privileged::windows::deployment::VERIFICATION_EXIT_CODE as i32
                        } else {
                            1
                        }
                    })
            });
        std::process::exit(code);
    }
    #[cfg(not(windows))]
    {
        eprintln!("The privileged service is available only on Windows");
    }
}
