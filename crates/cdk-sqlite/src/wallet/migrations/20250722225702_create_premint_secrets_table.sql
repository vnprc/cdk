-- Create premint_secrets table for storing PreMintSecrets with quote_id foreign key
CREATE TABLE IF NOT EXISTS premint_secrets (
    quote_id TEXT PRIMARY KEY,
    secrets TEXT NOT NULL,
    created_time INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    FOREIGN KEY(quote_id) REFERENCES mint_quote(id) ON DELETE CASCADE
);