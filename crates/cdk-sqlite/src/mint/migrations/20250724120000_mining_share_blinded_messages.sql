-- Add blinded_messages column to mint_quote table for mining shares
-- This stores the pre-generated blinded messages from the wallet for mining share quotes

ALTER TABLE mint_quote ADD COLUMN blinded_messages TEXT;