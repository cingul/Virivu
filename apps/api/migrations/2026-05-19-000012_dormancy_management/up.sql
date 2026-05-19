-- Add status and activity tracking to projects (studies)
ALTER TABLE projects 
ADD COLUMN status TEXT NOT NULL DEFAULT 'active',
ADD COLUMN last_activity_at TIMESTAMPTZ NOT NULL DEFAULT NOW();

-- Add status and activity tracking to sites
ALTER TABLE sites 
ADD COLUMN status TEXT NOT NULL DEFAULT 'active',
ADD COLUMN last_activity_at TIMESTAMPTZ NOT NULL DEFAULT NOW();

-- Create indices for scanning dormancy
CREATE INDEX IF NOT EXISTS idx_projects_status_activity ON projects(status, last_activity_at);
CREATE INDEX IF NOT EXISTS idx_sites_status_activity ON sites(status, last_activity_at);
