use std::cell::Cell;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use rand::rngs::OsRng;
use rand::RngCore;
use rusqlite::Connection;

use crate::microsoft_auth::{AccountRecord, AccountsRepository};

const TOKEN_ENCRYPTION_VERSION: u8 = 1;
const TOKEN_NONCE_LENGTH: usize = 12;
const TOKEN_KEY_LENGTH: usize = 32;
const TOKEN_KEY_ID: &str = "microsoft-token-key-v1";
const KEYRING_SERVICE_NAME: &str = "com.cubic.launcher";

pub trait SecretStore {
    fn get_secret(&self, key: &str) -> Result<Option<String>>;
    fn set_secret(&self, key: &str, secret: &str) -> Result<()>;
}

#[derive(Debug, Clone)]
pub struct KeyringSecretStore {
    service_name: String,
}

impl KeyringSecretStore {
    pub fn new() -> Self {
        Self {
            service_name: KEYRING_SERVICE_NAME.to_string(),
        }
    }
}

impl Default for KeyringSecretStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretStore for KeyringSecretStore {
    fn get_secret(&self, key: &str) -> Result<Option<String>> {
        let entry = keyring::Entry::new(&self.service_name, key)?;

        match entry.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(anyhow!(error.to_string())),
        }
    }

    fn set_secret(&self, key: &str, secret: &str) -> Result<()> {
        let entry = keyring::Entry::new(&self.service_name, key)?;
        entry
            .set_password(secret)
            .map_err(|error| anyhow!(error.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaintextAccountRecord {
    pub microsoft_id: String,
    pub xbox_gamertag: Option<String>,
    pub minecraft_uuid: Option<String>,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub profile_data: Option<String>,
    pub is_active: bool,
}

pub struct AccountTokenCipher<S> {
    secret_store: S,
    key_id: String,
    key_creation_blocked: Cell<bool>,
}

impl<S: SecretStore> AccountTokenCipher<S> {
    pub fn new(secret_store: S) -> Self {
        Self {
            secret_store,
            key_id: TOKEN_KEY_ID.to_string(),
            key_creation_blocked: Cell::new(false),
        }
    }

    pub fn encrypt_token(&self, token: &str) -> Result<Vec<u8>> {
        self.encrypt_token_with_key_creation(token, true)
    }

    fn encrypt_token_with_key_creation(
        &self,
        token: &str,
        create_key_if_missing: bool,
    ) -> Result<Vec<u8>> {
        let cipher = self.cipher(create_key_if_missing && !self.key_creation_blocked.get())?;
        let mut nonce_bytes = [0_u8; TOKEN_NONCE_LENGTH];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, token.as_bytes())
            .map_err(|_| anyhow!("failed to encrypt account token"))?;

        let mut payload = Vec::with_capacity(1 + TOKEN_NONCE_LENGTH + ciphertext.len());
        payload.push(TOKEN_ENCRYPTION_VERSION);
        payload.extend_from_slice(&nonce_bytes);
        payload.extend_from_slice(&ciphertext);

        Ok(payload)
    }

    pub fn decrypt_token(&self, payload: &[u8]) -> Result<String> {
        let cipher = match self.cipher(false) {
            Ok(cipher) => cipher,
            Err(error) => {
                self.key_creation_blocked.set(true);
                return Err(error);
            }
        };
        if payload.len() <= 1 + TOKEN_NONCE_LENGTH {
            bail!("encrypted token payload is too short");
        }

        if payload[0] != TOKEN_ENCRYPTION_VERSION {
            bail!("unsupported encrypted token payload version {}", payload[0]);
        }

        let nonce = Nonce::from_slice(&payload[1..1 + TOKEN_NONCE_LENGTH]);
        let ciphertext = &payload[1 + TOKEN_NONCE_LENGTH..];
        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|_| anyhow!("failed to decrypt account token"))?;

        String::from_utf8(plaintext).context("decrypted account token is not valid UTF-8")
    }

    fn cipher(&self, create_key_if_missing: bool) -> Result<Aes256Gcm> {
        let key_bytes = self.load_key(create_key_if_missing)?;
        let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
        Ok(Aes256Gcm::new(key))
    }

    fn load_key(&self, create_if_missing: bool) -> Result<[u8; TOKEN_KEY_LENGTH]> {
        if let Some(encoded_key) = self.secret_store.get_secret(&self.key_id)? {
            return decode_key_bytes(&encoded_key);
        }

        if !create_if_missing {
            bail!(
                "stored account credential key is unavailable; saved accounts cannot be decrypted and the account must be added again"
            );
        }

        let mut key_bytes = [0_u8; TOKEN_KEY_LENGTH];
        OsRng.fill_bytes(&mut key_bytes);
        self.secret_store
            .set_secret(&self.key_id, &STANDARD.encode(key_bytes))?;

        Ok(key_bytes)
    }
}

pub struct EncryptedAccountsRepository<'connection, S> {
    accounts_repository: AccountsRepository<'connection>,
    token_cipher: AccountTokenCipher<S>,
}

impl<'connection, S: SecretStore> EncryptedAccountsRepository<'connection, S> {
    pub fn new(connection: &'connection Connection, secret_store: S) -> Self {
        Self {
            accounts_repository: AccountsRepository::new(connection),
            token_cipher: AccountTokenCipher::new(secret_store),
        }
    }

    pub fn upsert_account(&self, account: &PlaintextAccountRecord) -> Result<()> {
        let create_key_if_missing = !self.accounts_repository.has_encrypted_account_tokens()?;
        self.accounts_repository.upsert_account(&AccountRecord {
            microsoft_id: account.microsoft_id.clone(),
            xbox_gamertag: account.xbox_gamertag.clone(),
            minecraft_uuid: account.minecraft_uuid.clone(),
            access_token_enc: account
                .access_token
                .as_deref()
                .map(|token| {
                    self.token_cipher
                        .encrypt_token_with_key_creation(token, create_key_if_missing)
                })
                .transpose()?,
            refresh_token_enc: account
                .refresh_token
                .as_deref()
                .map(|token| {
                    self.token_cipher
                        .encrypt_token_with_key_creation(token, create_key_if_missing)
                })
                .transpose()?,
            profile_data: account.profile_data.clone(),
            is_active: account.is_active,
        })
    }

    pub fn load_active_account(&self) -> Result<Option<PlaintextAccountRecord>> {
        self.accounts_repository
            .load_active_account()?
            .map(|account| self.decrypt_account(account))
            .transpose()
    }

    pub fn list_accounts(&self) -> Result<Vec<PlaintextAccountRecord>> {
        Ok(self
            .accounts_repository
            .list_accounts()?
            .into_iter()
            .filter_map(|account| self.decrypt_account(account).ok())
            .collect())
    }

    pub fn set_active_account(&self, microsoft_id: &str) -> Result<()> {
        self.accounts_repository.set_active_account(microsoft_id)
    }

    fn decrypt_account(&self, account: AccountRecord) -> Result<PlaintextAccountRecord> {
        Ok(PlaintextAccountRecord {
            microsoft_id: account.microsoft_id,
            xbox_gamertag: account.xbox_gamertag,
            minecraft_uuid: account.minecraft_uuid,
            access_token: account
                .access_token_enc
                .as_deref()
                .map(|payload| self.token_cipher.decrypt_token(payload))
                .transpose()?,
            refresh_token: account
                .refresh_token_enc
                .as_deref()
                .map(|payload| self.token_cipher.decrypt_token(payload))
                .transpose()?,
            profile_data: account.profile_data,
            is_active: account.is_active,
        })
    }
}

fn decode_key_bytes(encoded_key: &str) -> Result<[u8; TOKEN_KEY_LENGTH]> {
    let decoded = STANDARD
        .decode(encoded_key)
        .context("failed to decode stored token encryption key")?;

    if decoded.len() != TOKEN_KEY_LENGTH {
        bail!(
            "stored token encryption key must be {} bytes, got {}",
            TOKEN_KEY_LENGTH,
            decoded.len()
        );
    }

    let mut key_bytes = [0_u8; TOKEN_KEY_LENGTH];
    key_bytes.copy_from_slice(&decoded);
    Ok(key_bytes)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    use rusqlite::Connection;

    use crate::database::initialize_database;

    use super::{
        AccountTokenCipher, EncryptedAccountsRepository, PlaintextAccountRecord, SecretStore,
        TOKEN_ENCRYPTION_VERSION, TOKEN_NONCE_LENGTH,
    };

    fn unique_test_root() -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_nanos();

        env::temp_dir().join(format!("cubic-launcher-token-storage-test-{timestamp}"))
    }

    #[derive(Clone, Default)]
    struct MemorySecretStore {
        values: Arc<Mutex<HashMap<String, String>>>,
    }

    impl SecretStore for MemorySecretStore {
        fn get_secret(&self, key: &str) -> anyhow::Result<Option<String>> {
            Ok(self
                .values
                .lock()
                .expect("secret store mutex poisoned")
                .get(key)
                .cloned())
        }

        fn set_secret(&self, key: &str, secret: &str) -> anyhow::Result<()> {
            self.values
                .lock()
                .expect("secret store mutex poisoned")
                .insert(key.to_string(), secret.to_string());
            Ok(())
        }
    }

    #[test]
    fn encrypts_and_decrypts_token_roundtrip() {
        let cipher = AccountTokenCipher::new(MemorySecretStore::default());

        let encrypted = cipher
            .encrypt_token("access-token-value")
            .expect("token should encrypt");
        let decrypted = cipher
            .decrypt_token(&encrypted)
            .expect("token should decrypt");

        assert_ne!(encrypted, b"access-token-value");
        assert_eq!(decrypted, "access-token-value");
    }

    #[test]
    fn persisted_secret_store_key_allows_future_decryption() {
        let secret_store = MemorySecretStore::default();
        let first_cipher = AccountTokenCipher::new(secret_store.clone());
        let second_cipher = AccountTokenCipher::new(secret_store);

        let encrypted = first_cipher
            .encrypt_token("refresh-token-value")
            .expect("token should encrypt");
        let decrypted = second_cipher
            .decrypt_token(&encrypted)
            .expect("token should decrypt");

        assert_eq!(decrypted, "refresh-token-value");
    }

    #[test]
    fn encrypting_on_fresh_install_creates_credential_key() {
        let secret_store = MemorySecretStore::default();
        let cipher = AccountTokenCipher::new(secret_store.clone());

        let encrypted = cipher
            .encrypt_token("refresh-token-value")
            .expect("fresh install should create a credential key");

        assert_eq!(
            cipher
                .decrypt_token(&encrypted)
                .expect("created key should decrypt the token"),
            "refresh-token-value"
        );
        assert_eq!(
            secret_store
                .values
                .lock()
                .expect("secret store mutex poisoned")
                .len(),
            1
        );
    }

    #[test]
    fn missing_credential_key_does_not_orphan_existing_ciphertext() {
        let secret_store = MemorySecretStore::default();
        let encrypted = AccountTokenCipher::new(secret_store.clone())
            .encrypt_token("refresh-token-value")
            .expect("token should encrypt");
        secret_store
            .values
            .lock()
            .expect("secret store mutex poisoned")
            .clear();

        let cipher = AccountTokenCipher::new(secret_store.clone());
        let error = cipher
            .decrypt_token(&encrypted)
            .expect_err("missing key must not be silently replaced");

        assert!(error
            .to_string()
            .contains("saved accounts cannot be decrypted"));
        let encryption_error = cipher
            .encrypt_token("replacement-token")
            .expect_err("cipher must remember that saved ciphertext lost its key");
        assert!(encryption_error
            .to_string()
            .contains("saved accounts cannot be decrypted"));
        assert!(
            secret_store
                .values
                .lock()
                .expect("secret store mutex poisoned")
                .is_empty(),
            "decrypting existing ciphertext must not create a replacement key"
        );
    }

    #[test]
    fn encrypted_repository_refuses_replacement_key_for_saved_accounts() {
        let root_dir = unique_test_root();
        let database_path = root_dir.join("launcher_data.db");
        initialize_database(&database_path).expect("database should initialize");
        let connection = Connection::open(&database_path).expect("database should open");
        let secret_store = MemorySecretStore::default();
        let repository =
            EncryptedAccountsRepository::new(&connection, secret_store.clone());
        let account = PlaintextAccountRecord {
            microsoft_id: "account-a".into(),
            xbox_gamertag: Some("PlayerA".into()),
            minecraft_uuid: Some("uuid-a".into()),
            access_token: Some("access-before".into()),
            refresh_token: Some("refresh-before".into()),
            profile_data: None,
            is_active: true,
        };
        repository
            .upsert_account(&account)
            .expect("initial account should store");
        let original_blob: Vec<u8> = connection
            .query_row(
                "SELECT refresh_token_enc FROM accounts WHERE microsoft_id = ?1",
                ["account-a"],
                |row| row.get(0),
            )
            .expect("stored blob should load");
        secret_store
            .values
            .lock()
            .expect("secret store mutex poisoned")
            .clear();

        let error = repository
            .upsert_account(&PlaintextAccountRecord {
                access_token: Some("access-after".into()),
                refresh_token: Some("refresh-after".into()),
                ..account
            })
            .expect_err("saved ciphertext must prevent replacement-key creation");

        assert!(error
            .to_string()
            .contains("saved accounts cannot be decrypted"));
        assert!(
            secret_store
                .values
                .lock()
                .expect("secret store mutex poisoned")
                .is_empty(),
            "failed upsert must not create a replacement key"
        );
        let unchanged_blob: Vec<u8> = connection
            .query_row(
                "SELECT refresh_token_enc FROM accounts WHERE microsoft_id = ?1",
                ["account-a"],
                |row| row.get(0),
            )
            .expect("stored blob should remain");
        assert_eq!(unchanged_blob, original_blob);

        drop(repository);
        drop(connection);
        fs::remove_dir_all(&root_dir).expect("temporary root should be removable");
    }

    #[test]
    fn encrypted_repository_persists_ciphertext_and_loads_plaintext() {
        let root_dir = unique_test_root();
        let database_path = root_dir.join("launcher_data.db");

        initialize_database(&database_path).expect("database should initialize");
        let connection = Connection::open(&database_path).expect("database should open");
        let repository =
            EncryptedAccountsRepository::new(&connection, MemorySecretStore::default());

        repository
            .upsert_account(&PlaintextAccountRecord {
                microsoft_id: "account-a".into(),
                xbox_gamertag: Some("PlayerA".into()),
                minecraft_uuid: Some("uuid-a".into()),
                access_token: Some("access-a".into()),
                refresh_token: Some("refresh-a".into()),
                profile_data: Some("{\"name\":\"PlayerA\"}".into()),
                is_active: true,
            })
            .expect("account should store");

        let raw_row = connection
            .query_row(
                "SELECT access_token_enc, refresh_token_enc FROM accounts WHERE microsoft_id = ?1",
                ["account-a"],
                |row| {
                    Ok((
                        row.get::<_, Option<Vec<u8>>>(0)?,
                        row.get::<_, Option<Vec<u8>>>(1)?,
                    ))
                },
            )
            .expect("stored row should exist");
        let active_account = repository
            .load_active_account()
            .expect("active account should load")
            .expect("active account should exist");

        assert!(raw_row.0.is_some());
        assert!(raw_row.1.is_some());
        assert_ne!(raw_row.0.unwrap(), b"access-a");
        assert_ne!(raw_row.1.unwrap(), b"refresh-a");
        assert_eq!(active_account.access_token.as_deref(), Some("access-a"));
        assert_eq!(active_account.refresh_token.as_deref(), Some("refresh-a"));
        assert_eq!(active_account.xbox_gamertag.as_deref(), Some("PlayerA"));

        drop(connection);
        fs::remove_dir_all(&root_dir).expect("temporary root should be removable");
    }
    #[test]
    fn list_accounts_skips_corrupted_blob() {
        let root_dir = unique_test_root();
        let database_path = root_dir.join("launcher_data.db");

        initialize_database(&database_path).expect("database should initialize");
        let connection = Connection::open(&database_path).expect("database should open");
        let repository =
            EncryptedAccountsRepository::new(&connection, MemorySecretStore::default());

        repository
            .upsert_account(&PlaintextAccountRecord {
                microsoft_id: "account-valid".into(),
                xbox_gamertag: Some("ValidPlayer".into()),
                minecraft_uuid: Some("valid-uuid".into()),
                access_token: Some("valid-access-token".into()),
                refresh_token: Some("valid-refresh-token".into()),
                profile_data: None,
                is_active: false,
            })
            .expect("valid account should store");

        let mut corrupted_payload = vec![TOKEN_ENCRYPTION_VERSION];
        corrupted_payload.extend_from_slice(&[0; TOKEN_NONCE_LENGTH + 16]);
        connection
            .execute(
                r#"
                INSERT INTO accounts (
                    microsoft_id,
                    access_token_enc,
                    last_login,
                    is_active
                ) VALUES (?1, ?2, CURRENT_TIMESTAMP, FALSE)
                "#,
                rusqlite::params!["account-corrupted", corrupted_payload],
            )
            .expect("corrupted account should store");

        let accounts = repository
            .list_accounts()
            .expect("corrupted account should be skipped");

        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].microsoft_id, "account-valid");

        drop(connection);
        fs::remove_dir_all(&root_dir).expect("temporary root should be removable");
    }

}
