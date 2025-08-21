-- Add keyset_id column to mint_quote table for mining shares
-- This allows mining share quotes to store their predetermined keyset_id

ALTER TABLE mint_quote ADD COLUMN keyset_id TEXT;