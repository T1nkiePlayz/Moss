-- owned=0: the name was already on the server (added by hand) when a list asked for it, so Moss never removes it.
ALTER TABLE player_list_applied ADD COLUMN owned INTEGER NOT NULL DEFAULT 1;
