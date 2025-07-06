-- Add blinded_messages column to mint_quote table [NUT-XX eHash]
ALTER TABLE mint_quote ADD COLUMN blinded_messages TEXT;