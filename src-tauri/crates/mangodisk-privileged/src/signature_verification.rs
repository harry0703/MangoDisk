//! Preserve native trust failures across error context and the elevated process boundary.
use std::{error::Error, fmt, io};

pub const CERT_E_UNTRUSTEDROOT: u32 = 0x800b0109;

pub fn is_trust_status(code: u32) -> bool {
    matches!(code >> 16, 0x8009 | 0x800b)
}

#[derive(Debug)]
pub struct SignatureFailure(pub u32);

impl fmt::Display for SignatureFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Authenticode verification failed status=0x{:08X}",
            self.0
        )
    }
}

impl Error for SignatureFailure {}

pub fn native_code(error: &(dyn Error + 'static)) -> Option<u32> {
    if let Some(failure) = error.downcast_ref::<SignatureFailure>() {
        return Some(failure.0);
    }
    if let Some(error) = error.downcast_ref::<io::Error>() {
        return error.get_ref().and_then(|source| native_code(source));
    }
    error.source().and_then(native_code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_trust_statuses_from_process_crashes_and_setup_exit_codes() {
        for code in [CERT_E_UNTRUSTEDROOT, 0x800b0101, 0x80096010, 0x80092013] {
            assert!(is_trust_status(code));
        }
        for code in [0, 1, 13, 3010, 0xc0000005, 0xc0000409] {
            assert!(!is_trust_status(code));
        }
    }

    #[test]
    fn retains_the_native_status_through_installation_context() {
        for code in [CERT_E_UNTRUSTEDROOT, 0x800b0101, 0x80096010, 0x800b0100] {
            let signature = io::Error::new(io::ErrorKind::InvalidData, SignatureFailure(code));
            let installation = io::Error::new(io::ErrorKind::InvalidData, signature);
            assert_eq!(native_code(&installation), Some(code));
            assert!(installation.to_string().contains(&format!("0x{code:08X}")));
        }
    }

    #[test]
    fn does_not_classify_file_or_integrity_errors_as_signature_failures() {
        for error in [
            io::Error::from_raw_os_error(2),
            io::Error::new(io::ErrorKind::InvalidData, "component hash mismatch"),
        ] {
            assert_eq!(native_code(&error), None);
        }
    }
}
