//! One cross-process lock covers refresh rotation and each cart transaction.
use crate::{Error, Result, auth::Session};
use clap::ValueEnum;
use fs2::FileExt;
use serde_json::Value;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum CredentialStore {
    Keychain,
    File,
}
#[derive(Clone)]
pub struct Store {
    pub directory: PathBuf,
    pub backend: CredentialStore,
}
pub struct Guard {
    _file: File,
}
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self._file);
    }
}

impl Store {
    pub fn default_directory() -> Result<PathBuf> {
        directories::ProjectDirs::from("", "", "ametller-mcp")
            .map(|d| d.data_local_dir().to_owned())
            .ok_or(Error::Storage)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.directory.join(name)
    }
    fn prepare(&self) -> Result<()> {
        if self.directory.exists() {
            check_private(&self.directory, true)?;
        } else {
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                fs::DirBuilder::new()
                    .recursive(true)
                    .mode(0o700)
                    .create(&self.directory)
                    .map_err(|_| Error::Storage)?;
            }
            #[cfg(not(unix))]
            {
                return Err(Error::Input(
                    "This private storage backend requires macOS or Linux.",
                ));
            }
            check_private(&self.directory, true)?;
        }
        Ok(())
    }
    pub async fn lock(&self) -> Result<Guard> {
        self.prepare()?;
        let path = self.path("account.lock");
        reject_symlink(&path)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&path).map_err(|_| Error::Storage)?;
        check_private(&path, false)?;
        for _ in 0..100 {
            match file.try_lock_exclusive() {
                Ok(()) => return Ok(Guard { _file: file }),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    tokio::time::sleep(Duration::from_millis(50)).await
                }
                Err(_) => return Err(Error::Storage),
            }
        }
        Err(Error::Busy)
    }
    pub fn has_session(&self) -> Result<bool> {
        if !self.directory.exists() {
            return Ok(false);
        }
        check_private(&self.directory, true)?;
        Ok(self.read_value("connected.json")?.is_some())
    }
    pub fn metadata(&self) -> Result<Option<Value>> {
        self.read_value("connected.json")
    }
    #[cfg(any(target_os = "macos", test))]
    fn keychain_account(&self) -> Result<String> {
        let path = self.directory.canonicalize().map_err(|_| Error::Storage)?;
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(path.to_string_lossy().as_bytes());
        Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
    }
    fn validate_backend(&self) -> Result<()> {
        if let Some(metadata) = self.metadata()? {
            let requested = match self.backend {
                CredentialStore::File => "file",
                CredentialStore::Keychain => "keychain",
            };
            if metadata["credential_store"].as_str() != Some(requested) {
                return Err(Error::Input(
                    "This state directory uses another credential store. Select its original --credential-store, or use a different --state-dir.",
                ));
            }
        }
        Ok(())
    }
    pub fn load(&self) -> Result<Option<Session>> {
        self.validate_backend()?;
        let Some(metadata) = self.metadata()? else {
            return Ok(None);
        };
        let text = match self.backend {
            CredentialStore::File => self.read_bytes("credentials.json")?,
            CredentialStore::Keychain => {
                #[cfg(target_os = "macos")]
                {
                    Some(
                        security_framework::passwords::get_generic_password(
                            "ametller-mcp",
                            &self.keychain_account()?,
                        )
                        .map_err(|_| Error::Keychain)?,
                    )
                }
                #[cfg(not(target_os = "macos"))]
                {
                    return Err(Error::Input(
                        "Keychain is available only on macOS. Explicitly choose --credential-store file.",
                    ));
                }
            }
        };
        let session: Option<Session> = text
            .map(|bytes| serde_json::from_slice(&Zeroizing::new(bytes)).map_err(|_| Error::Storage))
            .transpose()?;
        // Credentials and metadata are separate writes. A failed account switch
        // must not let credentials for one account operate under another marker.
        if session.as_ref().is_some_and(|session| {
            metadata["customer_id"].as_str() != Some(session.customer_id.as_str())
        }) {
            return Err(Error::Input(
                "Stored credentials do not match the connected account. Sign in again with `ametller-mcp login`.",
            ));
        }
        Ok(session)
    }
    pub fn save(&self, session: &Session) -> Result<()> {
        self.validate_backend()?;
        let bytes = Zeroizing::new(serde_json::to_vec(session).map_err(|_| Error::Storage)?);
        match self.backend {
            CredentialStore::File => self.atomic("credentials.json", &bytes)?,
            CredentialStore::Keychain => {
                #[cfg(target_os = "macos")]
                {
                    security_framework::passwords::set_generic_password(
                        "ametller-mcp",
                        &self.keychain_account()?,
                        &bytes,
                    )
                    .map_err(|_| Error::Keychain)?;
                }
                #[cfg(not(target_os = "macos"))]
                {
                    return Err(Error::Input(
                        "Keychain is available only on macOS. Explicitly choose --credential-store file.",
                    ));
                }
            }
        }
        self.write_value("connected.json", &serde_json::json!({"customer_id":session.customer_id,"expires_at":session.expires_at,"credential_store":match self.backend {CredentialStore::File=>"file",CredentialStore::Keychain=>"keychain"}}))
    }
    pub fn delete(&self) -> Result<()> {
        self.validate_backend()?;
        match self.backend {
            CredentialStore::File => {
                self.remove("credentials.json")?;
                self.remove("connected.json")
            }
            CredentialStore::Keychain => {
                #[cfg(target_os = "macos")]
                {
                    self.delete_keychain_with(|account| {
                        security_framework::passwords::delete_generic_password(
                            "ametller-mcp",
                            account,
                        )
                        .map_err(|error| error.code())
                    })
                }
                #[cfg(not(target_os = "macos"))]
                {
                    Err(Error::Input(
                        "Keychain is available only on macOS. Explicitly choose --credential-store file.",
                    ))
                }
            }
        }
    }
    #[cfg(any(target_os = "macos", test))]
    fn delete_keychain_with(
        &self,
        delete: impl FnOnce(&str) -> std::result::Result<(), i32>,
    ) -> Result<()> {
        self.validate_backend()?;
        // A save can stop after writing the credential but before its marker.
        // Always delete this account; an already absent item is also logged out.
        match delete(&self.keychain_account()?) {
            Ok(()) | Err(-25300) => {} // errSecItemNotFound
            Err(_) => return Err(Error::Keychain),
        }
        self.remove("connected.json")
    }
    pub fn pending(&self) -> Result<Option<Value>> {
        self.read_value("pending-cart.json")
    }
    pub fn set_pending(&self, value: &Value) -> Result<()> {
        self.write_value("pending-cart.json", value)
    }
    pub fn clear_pending(&self) -> Result<()> {
        self.remove("pending-cart.json")
    }
    fn read_value(&self, name: &str) -> Result<Option<Value>> {
        self.read_bytes(name)?
            .map(|bytes| serde_json::from_slice(&bytes).map_err(|_| Error::Storage))
            .transpose()
    }
    fn read_bytes(&self, name: &str) -> Result<Option<Vec<u8>>> {
        let path = self.path(name);
        reject_symlink(&path)?;
        if !path.exists() {
            return Ok(None);
        }
        check_private(&path, false)?;
        let mut bytes = Vec::new();
        File::open(path)
            .map_err(|_| Error::Storage)?
            .take(65537)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::Storage)?;
        if bytes.len() > 65536 {
            return Err(Error::Storage);
        }
        Ok(Some(bytes))
    }
    fn write_value(&self, name: &str, value: &Value) -> Result<()> {
        self.atomic(
            name,
            &serde_json::to_vec(value).map_err(|_| Error::Storage)?,
        )
    }
    fn atomic(&self, name: &str, bytes: &[u8]) -> Result<()> {
        self.prepare()?;
        let path = self.path(name);
        reject_symlink(&path)?;
        if path.exists() {
            check_private(&path, false)?;
        }
        let temp = self.path(&format!(".{name}.{:016x}.tmp", rand::random::<u64>()));
        let result = (|| {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temp).map_err(|_| Error::Storage)?;
            file.write_all(bytes).map_err(|_| Error::Storage)?;
            file.sync_all().map_err(|_| Error::Storage)?;
            fs::rename(&temp, &path).map_err(|_| Error::Storage)?;
            File::open(&self.directory)
                .and_then(|directory| directory.sync_all())
                .map_err(|_| Error::Storage)
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result
    }
    fn remove(&self, name: &str) -> Result<()> {
        let path = self.path(name);
        reject_symlink(&path)?;
        if path.exists() {
            check_private(&path, false)?;
            fs::remove_file(path).map_err(|_| Error::Storage)?;
            File::open(&self.directory)
                .and_then(|d| d.sync_all())
                .map_err(|_| Error::Storage)?;
        }
        Ok(())
    }
}
fn reject_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(Error::Storage),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(Error::Storage),
    }
}
fn check_private(path: &Path, directory: bool) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|_| Error::Storage)?;
    if metadata.file_type().is_symlink()
        || metadata.is_dir() != directory
        || (!directory && !metadata.is_file())
    {
        return Err(Error::Storage);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.mode() & 0o077 != 0 {
            return Err(Error::Storage);
        }
        // The state directory is created by this process; an existing directory
        // must also belong to the same owner as the process's private home.
        let home = directories::BaseDirs::new().ok_or(Error::Storage)?;
        let owner = fs::metadata(home.home_dir())
            .map_err(|_| Error::Storage)?
            .uid();
        if metadata.uid() != owner {
            return Err(Error::Storage);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keychain_logout_deletes_credential_without_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store {
            directory: temp.path().join("state"),
            backend: CredentialStore::Keychain,
        };
        store.prepare().unwrap();
        let mut credentials = std::collections::HashSet::from([store.keychain_account().unwrap()]);
        assert!(!store.has_session().unwrap());
        store
            .delete_keychain_with(|account| {
                if credentials.remove(account) {
                    Ok(())
                } else {
                    Err(-25300)
                }
            })
            .unwrap();
        assert!(credentials.is_empty());
        assert!(!store.has_session().unwrap());
    }
    #[test]
    fn keychain_logout_removes_metadata_when_credential_is_already_absent() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store {
            directory: temp.path().join("state"),
            backend: CredentialStore::Keychain,
        };
        store
            .write_value(
                "connected.json",
                &serde_json::json!({"credential_store":"keychain"}),
            )
            .unwrap();
        let mut credentials = std::collections::HashSet::<String>::new();
        let mut called = false;
        store
            .delete_keychain_with(|account| {
                called = true;
                if credentials.remove(account) {
                    Ok(())
                } else {
                    Err(-25300)
                }
            })
            .unwrap();
        assert!(called);
        assert!(!store.has_session().unwrap());
    }
    #[test]
    fn keychain_logout_preserves_metadata_on_error_and_rejects_backend_mismatch() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store {
            directory: temp.path().join("state"),
            backend: CredentialStore::Keychain,
        };
        store
            .write_value(
                "connected.json",
                &serde_json::json!({"credential_store":"keychain"}),
            )
            .unwrap();
        assert!(matches!(
            store.delete_keychain_with(|_| Err(-25293)),
            Err(Error::Keychain)
        ));
        assert!(store.has_session().unwrap());
        store
            .write_value(
                "connected.json",
                &serde_json::json!({"credential_store":"file"}),
            )
            .unwrap();
        assert!(matches!(
            store.delete_keychain_with(|_| panic!("must not delete another backend's credential")),
            Err(Error::Input(_))
        ));
        assert_eq!(
            store.metadata().unwrap().unwrap()["credential_store"],
            "file"
        );
    }
    #[tokio::test]
    async fn private_store_atomic_roundtrip_and_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store {
            directory: temp.path().join("state"),
            backend: CredentialStore::File,
        };
        let _guard = store.lock().await.unwrap();
        store
            .set_pending(&serde_json::json!({"test":true}))
            .unwrap();
        assert_eq!(store.pending().unwrap().unwrap()["test"], true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::{PermissionsExt, symlink};
            assert_eq!(
                fs::metadata(&store.directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(store.path("pending-cart.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            store.clear_pending().unwrap();
            symlink(temp.path().join("outside"), store.path("pending-cart.json")).unwrap();
            assert!(store.set_pending(&serde_json::json!({})).is_err());
        }
    }
}
