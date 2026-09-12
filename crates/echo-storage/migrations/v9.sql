-- Applied after materializing independent copies for legacy shared content.
CREATE UNIQUE INDEX IF NOT EXISTS space_memberships_owner_idx ON space_memberships(saved_item_id);
