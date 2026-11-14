//! What outlives a session, behind one seam (ADR 0013 §7).
//!
//! Accounts and characters are read through [`Store`]: [`MemoryStore`] for
//! tests, a Postgres store later. The simulation step never reads the store:
//! the gateway reads an account's characters where it checks the password,
//! and they travel with the ticket (`World::admit`). Passwords exist only as
//! argon2id hashes. Checking one costs tens of milliseconds by design, so
//! [`login`] belongs where the gateway checks a password, never in a step.

use std::sync::{Mutex, MutexGuard};

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

/// What one accepted action changed, written as a whole or not at all
/// (ADR 0013 §7): a trade changes two characters at once. A character is
/// named by its name, which is unique.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Changes {
    pub characters: Vec<Character>,
}

pub trait Store {
    /// The account `username` names, with its password hash (PHC string).
    fn account(&self, username: &str) -> Option<(AccountId, String)>;
    /// The characters of `account`, in the order the lobby lists them.
    fn characters(&self, account: AccountId) -> Vec<Character>;
    /// Writes `changes` in one go; on an error nothing of it was written.
    fn commit(&self, changes: &Changes) -> Result<(), String>;
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
    accounts: Mutex<Vec<Account>>,
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
        let accounts = self.accounts.get_mut().expect("no store user panicked");
        accounts.push(Account {
            username: username.to_owned(),
            password_hash: hash_password(password),
            characters,
        });
        AccountId(accounts.len() as u32)
    }

    fn accounts(&self) -> MutexGuard<'_, Vec<Account>> {
        self.accounts.lock().expect("no store user panicked")
    }
}

impl Store for MemoryStore {
    fn account(&self, username: &str) -> Option<(AccountId, String)> {
        let accounts = self.accounts();
        let index = accounts.iter().position(|a| a.username == username)?;
        let account = AccountId(index as u32 + 1);
        Some((account, accounts[index].password_hash.clone()))
    }

    fn characters(&self, account: AccountId) -> Vec<Character> {
        let index = account.0.checked_sub(1).map(|i| i as usize);
        let accounts = self.accounts();
        index
            .and_then(|i| accounts.get(i))
            .map(|a| a.characters.clone())
            .unwrap_or_default()
    }

    fn commit(&self, changes: &Changes) -> Result<(), String> {
        let mut accounts = self.accounts();
        let mut slots = Vec::with_capacity(changes.characters.len());
        for changed in &changes.characters {
            let slot = accounts.iter().enumerate().find_map(|(a, account)| {
                let c = account
                    .characters
                    .iter()
                    .position(|c| c.name == changed.name)?;
                Some((a, c))
            });
            slots.push(slot.ok_or_else(|| format!("no character {}", changed.name))?);
        }
        for ((a, c), changed) in slots.into_iter().zip(&changes.characters) {
            accounts[a].characters[c] = changed.clone();
        }
        Ok(())
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

    fn named(name: &str) -> Character {
        Character {
            name: name.to_owned(),
            ref_id: 1907,
            scale: 34,
            level: 1,
            exp: 0,
            skill_exp: 0,
            gold: 0,
            skill_points: 0,
            strength: 20,
            intellect: 20,
            stat_points: 0,
            hp: 200,
            mp: 200,
            inventory_size: 45,
            masteries: Vec::new(),
            position: Position {
                region: 25000,
                x: 969.0,
                y: 0.0,
                z: 1419.0,
                heading: 0,
            },
        }
    }

    /// A commit is all or nothing: one character the store does not know
    /// keeps the other's change out too.
    #[test]
    fn a_commit_writes_all_of_it_or_nothing() {
        let mut store = MemoryStore::default();
        let admin = store.add_account("admin", "secret", vec![named("Hero")]);
        let mut moved = named("Hero");
        moved.position.x = 1119.0;
        let both = Changes {
            characters: vec![moved.clone(), named("Nobody")],
        };
        assert_eq!(store.commit(&both), Err("no character Nobody".to_owned()));
        assert_eq!(store.characters(admin), [named("Hero")]);
        let one = Changes {
            characters: vec![moved.clone()],
        };
        assert_eq!(store.commit(&one), Ok(()));
        assert_eq!(store.characters(admin), [moved]);
    }
}
