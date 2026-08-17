ALTER TABLE policies ADD COLUMN mode TEXT NOT NULL DEFAULT 'default';
ALTER TABLE policies ADD COLUMN allowed_categories TEXT NOT NULL DEFAULT '[]';
