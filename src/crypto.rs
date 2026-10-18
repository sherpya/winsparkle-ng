#[cfg(feature = "signature-ed25519")]
use crate::platform::KernelHandle;
use std::{fs::File, os::windows::fs::OpenOptionsExt, path::Path};
#[cfg(feature = "signature-ed25519")]
use std::{os::windows::io::AsRawHandle, ptr::null};

pub struct PreparedFile {
    // Retains the deny-write/delete handle through launch, including builds without crypto.
    #[cfg_attr(not(feature = "signature-ed25519"), allow(dead_code))]
    pub file: File,
}
impl PreparedFile {
    pub fn open(path: &Path) -> crate::Result<Self> {
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
        };
        // Deny write/delete sharing from verification until installer launch, and reject reparse points.
        let file = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .map_err(|e| e.to_string())?;
        use std::os::windows::fs::MetadataExt;
        let meta = file.metadata().map_err(|e| e.to_string())?;
        if !meta.is_file() || meta.file_attributes() & 0x400 != 0 {
            return Err("Installer is not a regular file".into());
        }
        Ok(Self { file })
    }
    #[cfg(feature = "signature-ed25519")]
    pub fn verify(&self, public: &str, signature: &str) -> crate::Result<()> {
        use base64::Engine;
        use windows_sys::Win32::System::Memory::*;
        let key = decode_key(public)?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(signature.trim())
            .map_err(|_| "Invalid Base64 Ed25519 signature")?;
        let signature = ed25519_dalek::Signature::from_slice(&bytes)
            .map_err(|_| "Invalid Ed25519 signature length")?;
        let size = usize::try_from(self.file.metadata().map_err(|e| e.to_string())?.len())
            .map_err(|_| "Installer is too large to map")?;
        if size == 0 {
            return key
                .verify_strict(&[], &signature)
                .map_err(|_| "Invalid update signature".into());
        }
        // SAFETY: owned file is locked against writes/deletion and lives longer than the read-only mapping.
        unsafe {
            let mapping = KernelHandle(CreateFileMappingW(
                self.file.as_raw_handle(),
                null(),
                PAGE_READONLY,
                0,
                0,
                null(),
            ));
            if mapping.0.is_null() {
                return Err(crate::platform::error("Cannot map installer"));
            }
            let view = MapViewOfFile(mapping.0, FILE_MAP_READ, 0, 0, 0);
            if view.Value.is_null() {
                return Err(crate::platform::error("Cannot map installer view"));
            }
            struct View(MEMORY_MAPPED_VIEW_ADDRESS);
            impl Drop for View {
                fn drop(&mut self) {
                    // SAFETY: this guard exclusively owns the mapped view.
                    unsafe {
                        UnmapViewOfFile(self.0);
                    }
                }
            }
            let view = View(view);
            let data = std::slice::from_raw_parts(view.0.Value.cast::<u8>(), size);
            key.verify_strict(data, &signature)
                .map_err(|_| "The update signature is missing or invalid".into())
        }
    }
    #[cfg(not(feature = "signature-ed25519"))]
    pub fn verify(&self, _: &str, _: &str) -> crate::Result<()> {
        Err("This build does not support Ed25519 verification".into())
    }
}
#[cfg(feature = "signature-ed25519")]
fn decode_key(public: &str) -> crate::Result<ed25519_dalek::VerifyingKey> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(public.trim())
        .map_err(|_| "Invalid Base64 Ed25519 key")?;
    let bytes: [u8; 32] = bytes.try_into().map_err(|_| "Invalid Ed25519 key length")?;
    ed25519_dalek::VerifyingKey::from_bytes(&bytes).map_err(|_| "Invalid Ed25519 key".into())
}
pub fn valid_key(public: &str) -> bool {
    #[cfg(feature = "signature-ed25519")]
    {
        decode_key(public).is_ok()
    }
    #[cfg(not(feature = "signature-ed25519"))]
    {
        let _ = public;
        false
    }
}

#[cfg(all(test, feature = "signature-ed25519"))]
mod tests {
    use super::*;
    use base64::Engine;
    use ed25519_dalek::Signer;
    #[test]
    fn mapped_signature_rejects_tampering_and_missing_signature() {
        let temp = crate::platform::TempDir::create().unwrap();
        let path = temp.path.join("update.exe");
        let signing = ed25519_dalek::SigningKey::from_bytes(&[7; 32]);
        let data = vec![42u8; 8 * 1024 * 1024];
        std::fs::write(&path, &data).unwrap();
        let key =
            base64::engine::general_purpose::STANDARD.encode(signing.verifying_key().as_bytes());
        let signature =
            base64::engine::general_purpose::STANDARD.encode(signing.sign(&data).to_bytes());
        let file = PreparedFile::open(&path).unwrap();
        file.verify(&key, &signature).unwrap();
        assert!(file.verify(&key, "").is_err());
        assert!(std::fs::write(&path, b"tampered").is_err());
        drop(file);
        std::fs::write(&path, b"tampered").unwrap();
        assert!(
            PreparedFile::open(&path)
                .unwrap()
                .verify(&key, &signature)
                .is_err()
        );
    }
}
