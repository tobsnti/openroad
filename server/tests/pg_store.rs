//! The Postgres store against a real database. Runs only with DATABASE_URL
//! (`cargo test -p server --test pg_store -- --ignored`), each test in a
//! schema of its own that is dropped afterwards.

use std::sync::atomic::{AtomicU32, Ordering};

use postgres::{Client, Config, NoTls};
use server::pg_store::PgStore;
use server::store::{login, Character, Position, Store};

const NEEDS: &str = "DATABASE_URL names the database to test against";

/// A schema of its own for one test, and a config whose connections use it.
struct Scratch {
    admin: Client,
    schema: String,
    config: Config,
}

impl Scratch {
    fn new() -> Scratch {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let url = std::env::var("DATABASE_URL").expect(NEEDS);
        let base: Config = url.parse().expect("DATABASE_URL is a connection string");
        let mut admin = base
            .connect(NoTls)
            .expect("the database accepts the connection");
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        let schema = format!("store_test_{}_{n}", std::process::id());
        admin
            .batch_execute(&format!("CREATE SCHEMA {schema}"))
            .unwrap();
        let mut config = base;
        config.options(&format!("-c search_path={schema}"));
        Scratch {
            admin,
            schema,
            config,
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let drop = format!("DROP SCHEMA {} CASCADE", self.schema);
        let _ = self.admin.batch_execute(&drop);
    }
}

fn hero() -> Character {
    Character {
        name: "Hero".to_owned(),
        ref_id: 1907,
        scale: 34,
        level: 10,
        exp: 120,
        skill_exp: 7,
        gold: 9_876_543_210,
        skill_points: 3,
        strength: 29,
        intellect: 31,
        stat_points: 3,
        hp: 489,
        mp: 512,
        inventory_size: 45,
        masteries: vec![(257, 1), (258, 0), (273, 4)],
        position: Position {
            region: 25000,
            x: 969.25,
            y: -0.5,
            z: 1419.75,
            heading: 0x4000,
        },
    }
}

/// What the seed puts in is what the seam reads back, field for field, and
/// the hash it stored checks the password.
#[test]
#[ignore = "needs a database: DATABASE_URL=<url> cargo test -p server --test pg_store -- --ignored"]
fn the_store_reads_back_what_the_seed_put_in() {
    let scratch = Scratch::new();
    let store = PgStore::open(&scratch.config).expect("the store opens");
    let mut kiro = hero();
    kiro.name = "Kiro".to_owned();
    kiro.masteries.clear();
    let admin = store
        .seed_account("admin", "secret", &[hero(), kiro.clone()])
        .unwrap();
    assert_eq!(store.characters(admin), [hero(), kiro]);
    assert_eq!(login(&store, "admin", "secret"), Some(admin));
    assert_eq!(login(&store, "admin", "Secret"), None);
    assert_eq!(store.account("nobody"), None);
}

/// Opening an up-to-date schema applies nothing, and seeding an account that
/// exists changes nothing.
#[test]
#[ignore = "needs a database: DATABASE_URL=<url> cargo test -p server --test pg_store -- --ignored"]
fn opening_twice_and_seeding_twice_change_nothing() {
    let mut scratch = Scratch::new();
    let first = PgStore::open(&scratch.config).expect("the store opens");
    let admin = first.seed_account("admin", "secret", &[hero()]).unwrap();
    let second = PgStore::open(&scratch.config).expect("the store opens again");
    let again = second.seed_account("admin", "other", &[]).unwrap();
    assert_eq!(again, admin);
    assert_eq!(second.characters(admin), [hero()]);
    assert_eq!(login(&second, "admin", "secret"), Some(admin));
    let sql = format!("SELECT count(*) FROM {}.schema_migrations", scratch.schema);
    let applied: i64 = scratch.admin.query_one(&sql, &[]).unwrap().get(0);
    assert_eq!(applied, 1);
}
