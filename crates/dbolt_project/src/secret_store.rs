use dbolt_database::{ConnectionId, SecretStore, StoreError};
use keyring::{Entry, Error as KeyringError};

const SERVICE: &str = "dbolt";

#[derive(Default)]
pub struct KeyringSecretStore;

impl KeyringSecretStore {
    pub fn new() -> Self {
        Self
    }

    fn entry(connection: ConnectionId, field: &str) -> Result<Entry, StoreError> {
        Entry::new(SERVICE, &format!("{connection}:{field}")).map_err(secret_error)
    }
}

fn secret_error(error: KeyringError) -> StoreError {
    StoreError::Secret(error.to_string())
}

impl SecretStore for KeyringSecretStore {
    fn get(&self, connection: ConnectionId, field: &str) -> Result<Option<String>, StoreError> {
        match Self::entry(connection, field)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(KeyringError::NoEntry) => Ok(None),
            Err(error) => Err(secret_error(error)),
        }
    }

    fn set(&self, connection: ConnectionId, field: &str, secret: &str) -> Result<(), StoreError> {
        Self::entry(connection, field)?
            .set_password(secret)
            .map_err(secret_error)
    }

    fn delete(&self, connection: ConnectionId, field: &str) -> Result<(), StoreError> {
        match Self::entry(connection, field)?.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(error) => Err(secret_error(error)),
        }
    }
}
