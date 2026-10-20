ALTER TABLE content_items ADD COLUMN source TEXT; -- NULL = installed by Moss; 'identified' = matched by file hash; 'modpack' = from a modpack
