-- Users and account records.
-- All monetary columns use NUMERIC — never FLOAT or DOUBLE PRECISION.

CREATE TABLE IF NOT EXISTS users (
    user_id     UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email       TEXT NOT NULL UNIQUE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    active      BOOLEAN NOT NULL DEFAULT true
);

CREATE TABLE IF NOT EXISTS accounts (
    account_id      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id         UUID NOT NULL REFERENCES users(user_id),
    broker          TEXT NOT NULL,           -- e.g. 'alpaca', 'coinbase'
    account_type    TEXT NOT NULL,           -- 'paper' | 'live'
    currency        TEXT NOT NULL DEFAULT 'USD',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    active          BOOLEAN NOT NULL DEFAULT true
);

CREATE TABLE IF NOT EXISTS account_balances (
    balance_id      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id      UUID NOT NULL REFERENCES accounts(account_id),
    currency        TEXT NOT NULL,
    available       NUMERIC(30, 10) NOT NULL,  -- never FLOAT
    total           NUMERIC(30, 10) NOT NULL,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
