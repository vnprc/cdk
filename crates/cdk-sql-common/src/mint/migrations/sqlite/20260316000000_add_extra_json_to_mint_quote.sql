-- Add extra_json column to mint_quote for custom payment method metadata
ALTER TABLE mint_quote ADD COLUMN extra_json TEXT;
