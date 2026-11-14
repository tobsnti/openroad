-- Accounts and characters, the read side of the store (S8). Unsigned wire
-- fields are stored in the next wider signed type and checked to their wire
-- range, so a value the client cannot receive cannot be stored either. The
-- two u64 fields (exp, gold) are BIGINT: values beyond 2^63 cannot be stored,
-- far above what the game reaches.

CREATE TABLE accounts (
    id            INTEGER GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    username      TEXT NOT NULL UNIQUE,
    -- argon2id, as a PHC string
    password_hash TEXT NOT NULL
);

CREATE TABLE characters (
    id             INTEGER GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    account_id     INTEGER NOT NULL REFERENCES accounts (id),
    name           TEXT NOT NULL UNIQUE,
    ref_id         BIGINT NOT NULL CHECK (ref_id BETWEEN 0 AND 4294967295),
    scale          SMALLINT NOT NULL CHECK (scale BETWEEN 0 AND 255),
    level          SMALLINT NOT NULL CHECK (level BETWEEN 0 AND 255),
    exp            BIGINT NOT NULL CHECK (exp >= 0),
    skill_exp      BIGINT NOT NULL CHECK (skill_exp BETWEEN 0 AND 4294967295),
    gold           BIGINT NOT NULL CHECK (gold >= 0),
    skill_points   BIGINT NOT NULL CHECK (skill_points BETWEEN 0 AND 4294967295),
    strength       INTEGER NOT NULL CHECK (strength BETWEEN 0 AND 65535),
    intellect      INTEGER NOT NULL CHECK (intellect BETWEEN 0 AND 65535),
    stat_points    INTEGER NOT NULL CHECK (stat_points BETWEEN 0 AND 65535),
    hp             BIGINT NOT NULL CHECK (hp BETWEEN 0 AND 4294967295),
    mp             BIGINT NOT NULL CHECK (mp BETWEEN 0 AND 4294967295),
    inventory_size SMALLINT NOT NULL CHECK (inventory_size BETWEEN 0 AND 255),
    region         INTEGER NOT NULL CHECK (region BETWEEN 0 AND 65535),
    x              REAL NOT NULL,
    y              REAL NOT NULL,
    z              REAL NOT NULL,
    heading        INTEGER NOT NULL CHECK (heading BETWEEN 0 AND 65535)
);

-- The lobby lists an account's characters in the order they were made.
CREATE INDEX characters_of_account ON characters (account_id, id);

CREATE TABLE masteries (
    character_id INTEGER NOT NULL REFERENCES characters (id) ON DELETE CASCADE,
    mastery_id   BIGINT NOT NULL CHECK (mastery_id BETWEEN 0 AND 4294967295),
    level        SMALLINT NOT NULL CHECK (level BETWEEN 0 AND 255),
    PRIMARY KEY (character_id, mastery_id)
);
