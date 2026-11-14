//! The store in PostgreSQL, behind the same seam as `MemoryStore` (ADR 0013
//! §7; the synchronous `postgres` client, decision 0009 addendum). The
//! simulation never calls it: the gateway's threads read accounts and
//! characters through [`Store`] (S8a). The schema is the versioned migrations
//! in `server/migrations`, applied in order when the store opens. Connection
//! settings come from the environment only.

use std::sync::Mutex;

use postgres::{Client, Config, GenericClient, NoTls};

use crate::store::{hash_password, AccountId, Changes, Character, Position, Store};

/// The migrations in order. A version once applied is never edited; a change
/// is a new version.
const MIGRATIONS: [(i32, &str); 1] = [(
    1,
    include_str!("../migrations/0001_accounts_and_characters.sql"),
)];

pub struct PgStore {
    /// One connection: the store is asked at the pace of logins, not steps.
    client: Mutex<Client>,
}

impl PgStore {
    /// Connects and brings the schema up to date.
    pub fn open(config: &Config) -> Result<PgStore, postgres::Error> {
        let mut client = config.connect(NoTls)?;
        migrate(&mut client)?;
        Ok(PgStore {
            client: Mutex::new(client),
        })
    }

    /// The admin seed: the account `username` with `password` (stored as an
    /// argon2id hash) and its characters, unless an account of that name
    /// exists already. Its id either way.
    pub fn seed_account(
        &self,
        username: &str,
        password: &str,
        characters: &[Character],
    ) -> Result<AccountId, postgres::Error> {
        let mut client = self.client.lock().expect("no store user panicked");
        let mut tx = client.transaction()?;
        let known = tx.query_opt("SELECT id FROM accounts WHERE username = $1", &[&username])?;
        if let Some(row) = known {
            return Ok(account_id(row.get(0)));
        }
        let hash = hash_password(password);
        let id: i32 = tx
            .query_one(
                "INSERT INTO accounts (username, password_hash) VALUES ($1, $2) RETURNING id",
                &[&username, &hash],
            )?
            .get(0);
        for c in characters {
            insert_character(&mut tx, id, c)?;
        }
        tx.commit()?;
        Ok(account_id(id))
    }
}

impl Store for PgStore {
    fn account(&self, username: &str) -> Option<(AccountId, String)> {
        let mut client = self.client.lock().expect("no store user panicked");
        let sql = "SELECT id, password_hash FROM accounts WHERE username = $1";
        match client.query_opt(sql, &[&username]) {
            Ok(row) => row.map(|r| (account_id(r.get(0)), r.get(1))),
            Err(e) => failed("an account", e, None),
        }
    }

    fn characters(&self, account: AccountId) -> Vec<Character> {
        let mut client = self.client.lock().expect("no store user panicked");
        read_characters(&mut *client, account)
            .unwrap_or_else(|e| failed("characters", e, Vec::new()))
    }

    /// One transaction: every changed character's row and masteries, or none
    /// (a transaction dropped before its commit is rolled back).
    fn commit(&self, changes: &Changes) -> Result<(), String> {
        let mut client = self.client.lock().expect("no store user panicked");
        let mut tx = client.transaction().map_err(|e| e.to_string())?;
        for c in &changes.characters {
            if !update_character(&mut tx, c).map_err(|e| e.to_string())? {
                return Err(format!("no character {}", c.name));
            }
        }
        tx.commit().map_err(|e| e.to_string())
    }
}

/// A read that failed is answered as nothing found, so a login is refused
/// rather than half served; the reason goes to the log.
fn failed<T>(what: &str, e: postgres::Error, nothing: T) -> T {
    eprintln!("store: reading {what} failed: {e}");
    nothing
}

fn migrate(client: &mut Client) -> Result<(), postgres::Error> {
    client.batch_execute(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
             version    INTEGER PRIMARY KEY,
             applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
         )",
    )?;
    for (version, sql) in MIGRATIONS {
        let mut tx = client.transaction()?;
        let applied = "SELECT 1 FROM schema_migrations WHERE version = $1";
        if tx.query_opt(applied, &[&version])?.is_none() {
            tx.batch_execute(sql)?;
            tx.execute(
                "INSERT INTO schema_migrations (version) VALUES ($1)",
                &[&version],
            )?;
        }
        tx.commit()?;
    }
    Ok(())
}

fn read_characters(
    client: &mut impl GenericClient,
    account: AccountId,
) -> Result<Vec<Character>, postgres::Error> {
    let account = i32::try_from(account.0).expect("account ids come from an INTEGER identity");
    let rows = client.query(
        "SELECT id, name, ref_id, scale, level, exp, skill_exp, gold, skill_points,
                strength, intellect, stat_points, hp, mp, inventory_size,
                region, x, y, z, heading
         FROM characters WHERE account_id = $1 ORDER BY id",
        &[&account],
    )?;
    let masteries = client.query(
        "SELECT m.character_id, m.mastery_id, m.level
         FROM masteries m JOIN characters c ON c.id = m.character_id
         WHERE c.account_id = $1 ORDER BY m.character_id, m.mastery_id",
        &[&account],
    )?;
    Ok(rows
        .iter()
        .map(|r| {
            let id: i32 = r.get("id");
            let of_this = masteries.iter().filter(|m| m.get::<_, i32>(0) == id);
            Character {
                name: r.get("name"),
                ref_id: wire(r.get::<_, i64>("ref_id")),
                scale: wire(r.get::<_, i16>("scale")),
                level: wire(r.get::<_, i16>("level")),
                exp: wire(r.get::<_, i64>("exp")),
                skill_exp: wire(r.get::<_, i64>("skill_exp")),
                gold: wire(r.get::<_, i64>("gold")),
                skill_points: wire(r.get::<_, i64>("skill_points")),
                strength: wire(r.get::<_, i32>("strength")),
                intellect: wire(r.get::<_, i32>("intellect")),
                stat_points: wire(r.get::<_, i32>("stat_points")),
                hp: wire(r.get::<_, i64>("hp")),
                mp: wire(r.get::<_, i64>("mp")),
                inventory_size: wire(r.get::<_, i16>("inventory_size")),
                masteries: of_this
                    .map(|m| (wire(m.get::<_, i64>(1)), wire(m.get::<_, i16>(2))))
                    .collect(),
                position: Position {
                    region: wire(r.get::<_, i32>("region")),
                    x: r.get("x"),
                    y: r.get("y"),
                    z: r.get("z"),
                    heading: wire(r.get::<_, i32>("heading")),
                },
            }
        })
        .collect())
}

fn insert_character(
    tx: &mut impl GenericClient,
    account: i32,
    c: &Character,
) -> Result<(), postgres::Error> {
    let at = c.position;
    let id: i32 = tx
        .query_one(
            "INSERT INTO characters (account_id, name, ref_id, scale, level, exp, skill_exp,
                 gold, skill_points, strength, intellect, stat_points, hp, mp,
                 inventory_size, region, x, y, z, heading)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15,
                 $16, $17, $18, $19, $20)
             RETURNING id",
            &[
                &account,
                &c.name,
                &i64::from(c.ref_id),
                &i16::from(c.scale),
                &i16::from(c.level),
                &db(c.exp),
                &i64::from(c.skill_exp),
                &db(c.gold),
                &i64::from(c.skill_points),
                &i32::from(c.strength),
                &i32::from(c.intellect),
                &i32::from(c.stat_points),
                &i64::from(c.hp),
                &i64::from(c.mp),
                &i16::from(c.inventory_size),
                &i32::from(at.region),
                &at.x,
                &at.y,
                &at.z,
                &i32::from(at.heading),
            ],
        )?
        .get(0);
    insert_masteries(tx, id, &c.masteries)
}

/// Writes a character's row and replaces its masteries; false when the store
/// has no character of that name.
fn update_character(tx: &mut impl GenericClient, c: &Character) -> Result<bool, postgres::Error> {
    let at = c.position;
    let row = tx.query_opt(
        "UPDATE characters SET ref_id = $2, scale = $3, level = $4, exp = $5, skill_exp = $6,
             gold = $7, skill_points = $8, strength = $9, intellect = $10, stat_points = $11,
             hp = $12, mp = $13, inventory_size = $14, region = $15, x = $16, y = $17, z = $18,
             heading = $19
         WHERE name = $1 RETURNING id",
        &[
            &c.name,
            &i64::from(c.ref_id),
            &i16::from(c.scale),
            &i16::from(c.level),
            &db(c.exp),
            &i64::from(c.skill_exp),
            &db(c.gold),
            &i64::from(c.skill_points),
            &i32::from(c.strength),
            &i32::from(c.intellect),
            &i32::from(c.stat_points),
            &i64::from(c.hp),
            &i64::from(c.mp),
            &i16::from(c.inventory_size),
            &i32::from(at.region),
            &at.x,
            &at.y,
            &at.z,
            &i32::from(at.heading),
        ],
    )?;
    let Some(row) = row else {
        return Ok(false);
    };
    let id: i32 = row.get(0);
    tx.execute("DELETE FROM masteries WHERE character_id = $1", &[&id])?;
    insert_masteries(tx, id, &c.masteries)?;
    Ok(true)
}

fn insert_masteries(
    tx: &mut impl GenericClient,
    character: i32,
    masteries: &[(u32, u8)],
) -> Result<(), postgres::Error> {
    for (mastery, level) in masteries {
        tx.execute(
            "INSERT INTO masteries (character_id, mastery_id, level) VALUES ($1, $2, $3)",
            &[&character, &i64::from(*mastery), &i16::from(*level)],
        )?;
    }
    Ok(())
}

fn account_id(id: i32) -> AccountId {
    AccountId(u32::try_from(id).expect("identity ids are positive"))
}

/// A stored value back in its wire type; the schema's checks keep it in range.
fn wire<T: TryFrom<S>, S: Copy + std::fmt::Display>(v: S) -> T {
    T::try_from(v)
        .unwrap_or_else(|_| panic!("{v} is out of its wire range, which the schema checks"))
}

/// A u64 for a BIGINT column; values beyond 2^63 are far above the game's.
fn db(v: u64) -> i64 {
    i64::try_from(v).expect("the game stays far below 2^63")
}
