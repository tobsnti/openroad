//! What outlives a session, behind one seam (ADR 0013 §7).
//!
//! Accounts and characters are read through [`Store`]: [`MemoryStore`] for
//! tests, a Postgres store later, which is then queried off the simulation
//! step. Passwords exist only as argon2id hashes. Checking one costs tens of
//! milliseconds by design, so [`login`] belongs where the gateway checks a
//! password, never in a step.

use argon2::password_hash::{PasswordHash, SaltString};
use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use rand_core::OsRng;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AccountId(pub u32);

/// A character as the store keeps it.
#[derive(Clone, Debug, PartialEq)]
pub struct Character {
    pub name: String,
    /// The characterdata row of the character's model.
    pub ref_id: u32,
    pub scale: u8,
    pub level: u8,
    pub exp: u64,
    pub skill_exp: u32,
    pub gold: u64,
    pub skill_points: u32,
    pub strength: u16,
    pub intellect: u16,
    pub stat_points: u16,
    pub hp: u32,
    pub mp: u32,
    /// Slots in the inventory.
    pub inventory_size: u8,
    /// Mastery id and level.
    pub masteries: Vec<(u32, u8)>,
    pub position: Position,
}

/// Where a character stands: region, coordinates within it, and heading.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Position {
    pub region: u16,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub heading: u16,
}

pub trait Store {
    /// The account `username` names, with its password hash (PHC string).
    fn account(&self, username: &str) -> Option<(AccountId, String)>;
    /// The characters of `account`, in the order the lobby lists them.
    fn characters(&self, account: AccountId) -> Vec<Character>;
}

/// The account `username` names, if `password` is its password.
pub fn login(store: &dyn Store, username: &str, password: &str) -> Option<AccountId> {
    let (account, hash) = store.account(username)?;
    let hash = PasswordHash::new(&hash).ok()?;
    let checked = Argon2::default().verify_password(password.as_bytes(), &hash);
    checked.is_ok().then_some(account)
}

/// `password` as an argon2id hash with a fresh salt.
pub fn hash_password(password: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .expect("argon2id with its default parameters hashes any password")
        .to_string()
}

#[derive(Default)]
pub struct MemoryStore {
    accounts: Vec<Account>,
}

struct Account {
    username: String,
    password_hash: String,
    characters: Vec<Character>,
}

impl MemoryStore {
    /// Add an account, as the admin seed of a fresh store does.
    pub fn add_account(
        &mut self,
        username: &str,
        password: &str,
        characters: Vec<Character>,
    ) -> AccountId {
        self.accounts.push(Account {
            username: username.to_owned(),
            password_hash: hash_password(password),
            characters,
        });
        AccountId(self.accounts.len() as u32)
    }

    fn get(&self, account: AccountId) -> Option<&Account> {
        self.accounts.get(account.0.checked_sub(1)? as usize)
    }
}

impl Store for MemoryStore {
    fn account(&self, username: &str) -> Option<(AccountId, String)> {
        let index = self.accounts.iter().position(|a| a.username == username)?;
        let account = AccountId(index as u32 + 1);
        Some((account, self.accounts[index].password_hash.clone()))
    }

    fn characters(&self, account: AccountId) -> Vec<Character> {
        self.get(account)
            .map(|a| a.characters.clone())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_password_logs_in() {
        let mut store = MemoryStore::default();
        let admin = store.add_account("admin", "secret", Vec::new());
        assert_eq!(login(&store, "admin", "secret"), Some(admin));
        assert_eq!(login(&store, "admin", "Secret"), None);
        assert_eq!(login(&store, "nobody", "secret"), None);
        let (_, hash) = store.account("admin").expect("the account exists");
        assert!(hash.starts_with("$argon2id$"), "stored as {hash}");
    }
}
