-- Applied atomically by the existing storage writer. No payload changes.
CREATE TABLE spaces (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 kind TEXT NOT NULL CHECK(kind IN ('history','favorites','collection')),
 title TEXT NOT NULL,
 normalized_title TEXT NOT NULL UNIQUE,
 icon_key TEXT,
 description TEXT NOT NULL DEFAULT '',
 order_key INTEGER NOT NULL,
 revision INTEGER NOT NULL DEFAULT 1 CHECK(revision>0),
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL,
 CHECK((id=1 AND kind='history' AND order_key=0) OR
       (id=2 AND kind='favorites' AND order_key=1) OR
       (id>=3 AND kind='collection' AND order_key>=2))
);
CREATE INDEX spaces_order_idx ON spaces(order_key,id);
INSERT INTO spaces(id,kind,title,normalized_title,icon_key,order_key,created_at,updated_at)
 VALUES (1,'history','History','history',NULL,0,unixepoch(),unixepoch()),
        (2,'favorites','Favorites','favorites',NULL,1,unixepoch(),unixepoch());
CREATE TABLE space_memberships (
 space_id INTEGER NOT NULL REFERENCES spaces(id) ON DELETE CASCADE CHECK(space_id<>1),
 saved_item_id INTEGER NOT NULL REFERENCES saved_items(id) ON DELETE CASCADE,
 sort_key INTEGER NOT NULL,
 created_at INTEGER NOT NULL,
 PRIMARY KEY(space_id,saved_item_id)
);
CREATE INDEX space_memberships_page_idx ON space_memberships(space_id,sort_key,saved_item_id);
CREATE INDEX space_memberships_saved_idx ON space_memberships(saved_item_id,space_id);
INSERT INTO space_memberships(space_id,saved_item_id,sort_key,created_at)
 SELECT 2,id,(ROW_NUMBER() OVER(ORDER BY favorite_order,id)-1)*1024,unixepoch()
 FROM saved_items;
ALTER TABLE clipboard_settings ADD COLUMN ui_settings_json TEXT NOT NULL DEFAULT '{"version":1}';
ALTER TABLE clipboard_settings ADD COLUMN settings_revision INTEGER NOT NULL DEFAULT 1;
PRAGMA user_version=6;
