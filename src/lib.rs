use std::time::{SystemTime, UNIX_EPOCH};

use craftsql_core::PageStore;
use rusqlite::{Connection, OpenFlags};
use thiserror::Error;

pub type InodeId = i64;

#[derive(Error, Debug)]
pub enum CraftVfsError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("VFS Register error: {0}")]
    VfsRegister(String),
    #[error("Path not found: {0}")]
    PathNotFound(String),
    #[error("File already exists: {0}")]
    FileExists(String),
    #[error("Directory not empty")]
    DirectoryNotEmpty,
    #[error("Not a directory")]
    NotADirectory,
    #[error("Not a file")]
    NotAFile,
    #[error("Invalid path")]
    InvalidPath,
}

pub type Result<T> = std::result::Result<T, CraftVfsError>;

#[derive(Debug, Clone, PartialEq)]
pub enum FileType {
    File,
    Dir,
    Symlink,
}

impl FileType {
    fn from_str(s: &str) -> Self {
        match s {
            "file" => FileType::File,
            "dir" => FileType::Dir,
            "symlink" => FileType::Symlink,
            _ => FileType::File, // default
        }
    }
}

#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub inode_id: InodeId,
    pub file_type: FileType,
}

#[derive(Debug, Clone)]
pub struct InodeStat {
    pub id: InodeId,
    pub size: u64,
    pub file_type: FileType,
    pub created_at: i64,
    pub modified_at: i64,
}

pub struct CraftVfs {
    db: Connection,
}

impl CraftVfs {
    /// Open/create a CraftVFS filesystem backed by the given PageStore
    pub fn open(store: impl PageStore + 'static) -> Result<Self> {
        // Register the CraftSQL VFS
        craftsql_vfs::register("craftsql", store)
            .map_err(|e| CraftVfsError::VfsRegister(e.to_string()))?;

        // Open connection with the CraftSQL VFS
        let db = Connection::open_with_flags_and_vfs(
            ":memory:", // Not used - the VFS handles storage
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
            "craftsql",
        )?;

        // Enable foreign keys
        db.execute("PRAGMA foreign_keys = ON", [])?;

        let vfs = CraftVfs { db };
        vfs.initialize_schema()?;
        Ok(vfs)
    }

    fn initialize_schema(&self) -> Result<()> {
        // Check if schema already exists
        let table_exists: bool = self.db.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='inodes'",
            [],
            |row| Ok(row.get::<_, i64>(0)? > 0),
        )?;

        if !table_exists {
            // Create schema
            self.db.execute(
                r#"
                CREATE TABLE inodes (
                    id          INTEGER PRIMARY KEY AUTOINCREMENT,
                    size        INTEGER NOT NULL DEFAULT 0,
                    type        TEXT NOT NULL,
                    owner       TEXT NOT NULL DEFAULT '',
                    permissions INTEGER NOT NULL DEFAULT 493,  -- 0o755
                    created_at  INTEGER NOT NULL,
                    modified_at INTEGER NOT NULL
                )
                "#,
                [],
            )?;

            self.db.execute(
                r#"
                CREATE TABLE dirents (
                    parent_id   INTEGER NOT NULL,
                    name        TEXT NOT NULL,
                    inode_id    INTEGER NOT NULL,
                    PRIMARY KEY (parent_id, name),
                    FOREIGN KEY (parent_id) REFERENCES inodes(id),
                    FOREIGN KEY (inode_id) REFERENCES inodes(id)
                )
                "#,
                [],
            )?;

            self.db.execute(
                r#"
                CREATE TABLE file_data (
                    inode_id    INTEGER PRIMARY KEY,
                    content     BLOB NOT NULL DEFAULT X'',
                    FOREIGN KEY (inode_id) REFERENCES inodes(id)
                )
                "#,
                [],
            )?;

            // Create root directory (inode 1)
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;

            self.db.execute(
                r#"
                INSERT INTO inodes (id, size, type, owner, permissions, created_at, modified_at)
                VALUES (1, 0, 'dir', '', 493, ?, ?)
                "#,
                rusqlite::params![now, now],
            )?;
        }

        Ok(())
    }

    fn current_timestamp() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    /// Create a directory
    pub fn mkdir(&self, parent: InodeId, name: &str) -> Result<InodeId> {
        // Validate parent is a directory
        let parent_type: String = self.db.query_row(
            "SELECT type FROM inodes WHERE id = ?",
            rusqlite::params![parent],
            |row| row.get(0),
        )?;

        if parent_type != "dir" {
            return Err(CraftVfsError::NotADirectory);
        }

        // Check if name already exists in parent
        let exists: bool = self.db.query_row(
            "SELECT COUNT(*) FROM dirents WHERE parent_id = ? AND name = ?",
            rusqlite::params![parent, name],
            |row| Ok(row.get::<_, i64>(0)? > 0),
        ).unwrap_or(false);

        if exists {
            return Err(CraftVfsError::FileExists(name.to_string()));
        }

        // Create inode
        let now = Self::current_timestamp();
        self.db.execute(
            "INSERT INTO inodes (size, type, owner, permissions, created_at, modified_at) VALUES (0, 'dir', '', 493, ?, ?)",
            rusqlite::params![now, now],
        )?;

        let inode_id = self.db.last_insert_rowid();

        // Create dirent
        self.db.execute(
            "INSERT INTO dirents (parent_id, name, inode_id) VALUES (?, ?, ?)",
            rusqlite::params![parent, name, inode_id],
        )?;

        // Update parent's modified time
        self.db.execute(
            "UPDATE inodes SET modified_at = ? WHERE id = ?",
            rusqlite::params![now, parent],
        )?;

        Ok(inode_id)
    }

    /// Create a file (empty)
    pub fn create_file(&self, parent: InodeId, name: &str) -> Result<InodeId> {
        // Validate parent is a directory
        let parent_type: String = self.db.query_row(
            "SELECT type FROM inodes WHERE id = ?",
            rusqlite::params![parent],
            |row| row.get(0),
        )?;

        if parent_type != "dir" {
            return Err(CraftVfsError::NotADirectory);
        }

        // Check if name already exists in parent
        let exists: bool = self.db.query_row(
            "SELECT COUNT(*) FROM dirents WHERE parent_id = ? AND name = ?",
            rusqlite::params![parent, name],
            |row| Ok(row.get::<_, i64>(0)? > 0),
        ).unwrap_or(false);

        if exists {
            return Err(CraftVfsError::FileExists(name.to_string()));
        }

        // Create inode
        let now = Self::current_timestamp();
        self.db.execute(
            "INSERT INTO inodes (size, type, owner, permissions, created_at, modified_at) VALUES (0, 'file', '', 420, ?, ?)", // 0o644
            rusqlite::params![now, now],
        )?;

        let inode_id = self.db.last_insert_rowid();

        // Create dirent
        self.db.execute(
            "INSERT INTO dirents (parent_id, name, inode_id) VALUES (?, ?, ?)",
            rusqlite::params![parent, name, inode_id],
        )?;

        // Create empty file data entry
        self.db.execute(
            "INSERT INTO file_data (inode_id, content) VALUES (?, X'')",
            rusqlite::params![inode_id],
        )?;

        // Update parent's modified time
        self.db.execute(
            "UPDATE inodes SET modified_at = ? WHERE id = ?",
            rusqlite::params![now, parent],
        )?;

        Ok(inode_id)
    }

    /// Write file content
    pub fn write(&self, inode: InodeId, data: &[u8]) -> Result<()> {
        // Validate inode is a file
        let inode_type: String = self.db.query_row(
            "SELECT type FROM inodes WHERE id = ?",
            rusqlite::params![inode],
            |row| row.get(0),
        )?;

        if inode_type != "file" {
            return Err(CraftVfsError::NotAFile);
        }

        let now = Self::current_timestamp();

        // Update file content
        self.db.execute(
            "UPDATE file_data SET content = ? WHERE inode_id = ?",
            rusqlite::params![data, inode],
        )?;

        // Update inode size and modified time
        self.db.execute(
            "UPDATE inodes SET size = ?, modified_at = ? WHERE id = ?",
            rusqlite::params![data.len() as i64, now, inode],
        )?;

        Ok(())
    }

    /// Read file content
    pub fn read(&self, inode: InodeId) -> Result<Vec<u8>> {
        // Validate inode is a file
        let inode_type: String = self.db.query_row(
            "SELECT type FROM inodes WHERE id = ?",
            rusqlite::params![inode],
            |row| row.get(0),
        )?;

        if inode_type != "file" {
            return Err(CraftVfsError::NotAFile);
        }

        let content: Vec<u8> = self.db.query_row(
            "SELECT content FROM file_data WHERE inode_id = ?",
            rusqlite::params![inode],
            |row| row.get(0),
        )?;

        Ok(content)
    }

    /// List directory entries
    pub fn read_dir(&self, inode: InodeId) -> Result<Vec<DirEntry>> {
        // Validate inode is a directory
        let inode_type: String = self.db.query_row(
            "SELECT type FROM inodes WHERE id = ?",
            rusqlite::params![inode],
            |row| row.get(0),
        )?;

        if inode_type != "dir" {
            return Err(CraftVfsError::NotADirectory);
        }

        let mut stmt = self.db.prepare(
            r#"
            SELECT d.name, d.inode_id, i.type
            FROM dirents d
            JOIN inodes i ON d.inode_id = i.id
            WHERE d.parent_id = ?
            ORDER BY d.name
            "#,
        )?;

        let entries = stmt
            .query_map(rusqlite::params![inode], |row| {
                Ok(DirEntry {
                    name: row.get(0)?,
                    inode_id: row.get(1)?,
                    file_type: FileType::from_str(&row.get::<_, String>(2)?),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(entries)
    }

    /// Stat an inode
    pub fn stat(&self, inode: InodeId) -> Result<InodeStat> {
        let stat = self.db.query_row(
            "SELECT id, size, type, created_at, modified_at FROM inodes WHERE id = ?",
            rusqlite::params![inode],
            |row| {
                Ok(InodeStat {
                    id: row.get(0)?,
                    size: row.get(1)?,
                    file_type: FileType::from_str(&row.get::<_, String>(2)?),
                    created_at: row.get(3)?,
                    modified_at: row.get(4)?,
                })
            },
        )?;

        Ok(stat)
    }

    /// Lookup a name in a directory
    pub fn lookup(&self, parent: InodeId, name: &str) -> Result<InodeId> {
        let inode_id: InodeId = self.db.query_row(
            "SELECT inode_id FROM dirents WHERE parent_id = ? AND name = ?",
            rusqlite::params![parent, name],
            |row| row.get(0),
        )?;

        Ok(inode_id)
    }

    /// Rename/move entry
    pub fn rename(&self, from_parent: InodeId, from_name: &str, to_parent: InodeId, to_name: &str) -> Result<()> {
        // Get the inode being renamed (to validate it exists)
        let _inode_id = self.lookup(from_parent, from_name)?;

        // Check if target already exists
        let target_exists = self.lookup(to_parent, to_name).is_ok();
        if target_exists {
            return Err(CraftVfsError::FileExists(to_name.to_string()));
        }

        let now = Self::current_timestamp();

        // Update the dirent (this is O(1))
        self.db.execute(
            "UPDATE dirents SET parent_id = ?, name = ? WHERE parent_id = ? AND name = ?",
            rusqlite::params![to_parent, to_name, from_parent, from_name],
        )?;

        // Update modified times
        self.db.execute(
            "UPDATE inodes SET modified_at = ? WHERE id IN (?, ?)",
            rusqlite::params![now, from_parent, to_parent],
        )?;

        Ok(())
    }

    /// Remove a file
    pub fn remove_file(&self, parent: InodeId, name: &str) -> Result<()> {
        let inode_id = self.lookup(parent, name)?;

        // Validate it's a file
        let file_type: String = self.db.query_row(
            "SELECT type FROM inodes WHERE id = ?",
            rusqlite::params![inode_id],
            |row| row.get(0),
        )?;

        if file_type != "file" {
            return Err(CraftVfsError::NotAFile);
        }

        let now = Self::current_timestamp();

        // Remove file data
        self.db.execute("DELETE FROM file_data WHERE inode_id = ?", rusqlite::params![inode_id])?;

        // Remove dirent
        self.db.execute(
            "DELETE FROM dirents WHERE parent_id = ? AND name = ?",
            rusqlite::params![parent, name],
        )?;

        // Remove inode
        self.db.execute("DELETE FROM inodes WHERE id = ?", rusqlite::params![inode_id])?;

        // Update parent's modified time
        self.db.execute(
            "UPDATE inodes SET modified_at = ? WHERE id = ?",
            rusqlite::params![now, parent],
        )?;

        Ok(())
    }

    /// Remove an empty directory
    pub fn remove_dir(&self, parent: InodeId, name: &str) -> Result<()> {
        let inode_id = self.lookup(parent, name)?;

        // Validate it's a directory
        let file_type: String = self.db.query_row(
            "SELECT type FROM inodes WHERE id = ?",
            rusqlite::params![inode_id],
            |row| row.get(0),
        )?;

        if file_type != "dir" {
            return Err(CraftVfsError::NotADirectory);
        }

        // Check if directory is empty
        let child_count: i64 = self.db.query_row(
            "SELECT COUNT(*) FROM dirents WHERE parent_id = ?",
            rusqlite::params![inode_id],
            |row| row.get(0),
        )?;

        if child_count > 0 {
            return Err(CraftVfsError::DirectoryNotEmpty);
        }

        let now = Self::current_timestamp();

        // Remove dirent
        self.db.execute(
            "DELETE FROM dirents WHERE parent_id = ? AND name = ?",
            rusqlite::params![parent, name],
        )?;

        // Remove inode
        self.db.execute("DELETE FROM inodes WHERE id = ?", rusqlite::params![inode_id])?;

        // Update parent's modified time
        self.db.execute(
            "UPDATE inodes SET modified_at = ? WHERE id = ?",
            rusqlite::params![now, parent],
        )?;

        Ok(())
    }

    /// Resolve a path string to an inode (convenience)
    pub fn resolve_path(&self, path: &str) -> Result<InodeId> {
        if path.is_empty() || path == "/" {
            return Ok(1); // root
        }

        let path = path.trim_start_matches('/');
        let components: Vec<&str> = path.split('/').filter(|c| !c.is_empty()).collect();

        let mut current_inode = 1i64; // root

        for component in components {
            current_inode = self.lookup(current_inode, component)?;
        }

        Ok(current_inode)
    }

    /// Snapshot current state
    pub fn snapshot(&self, _name: &str) -> Result<()> {
        // For now, this is a no-op as snapshots would be handled by the underlying PageStore
        // In a full implementation, this would create a snapshot through the CraftSQL interface
        Ok(())
    }

    /// List snapshots
    pub fn list_snapshots(&self) -> Result<Vec<String>> {
        // For now, return empty as snapshots would be handled by the underlying PageStore
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craftsql_store_local::LocalPageStore;

    fn create_test_vfs() -> CraftVfs {
        let temp_dir = std::env::temp_dir().join(format!("craftvfs-test-{}", std::process::id()));
        let store = LocalPageStore::new(&temp_dir).expect("Failed to create test store");
        CraftVfs::open(store).expect("Failed to create VFS")
    }

    #[test]
    fn test_root_directory_exists() {
        let vfs = create_test_vfs();
        let stat = vfs.stat(1).expect("Failed to stat root");
        assert_eq!(stat.id, 1);
        assert_eq!(stat.file_type, FileType::Dir);
    }

    #[test]
    fn test_create_directory() {
        let vfs = create_test_vfs();
        let dir_id = vfs.mkdir(1, "testdir").expect("Failed to create directory");
        
        let stat = vfs.stat(dir_id).expect("Failed to stat directory");
        assert_eq!(stat.file_type, FileType::Dir);

        let entries = vfs.read_dir(1).expect("Failed to read root directory");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "testdir");
        assert_eq!(entries[0].inode_id, dir_id);
        assert_eq!(entries[0].file_type, FileType::Dir);
    }

    #[test]
    fn test_create_file() {
        let vfs = create_test_vfs();
        let file_id = vfs.create_file(1, "testfile").expect("Failed to create file");
        
        let stat = vfs.stat(file_id).expect("Failed to stat file");
        assert_eq!(stat.file_type, FileType::File);
        assert_eq!(stat.size, 0);

        let entries = vfs.read_dir(1).expect("Failed to read root directory");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "testfile");
        assert_eq!(entries[0].inode_id, file_id);
        assert_eq!(entries[0].file_type, FileType::File);
    }

    #[test]
    fn test_write_read_file() {
        let vfs = create_test_vfs();
        let file_id = vfs.create_file(1, "testfile").expect("Failed to create file");
        
        let data = b"Hello, CraftVFS!";
        vfs.write(file_id, data).expect("Failed to write file");

        let stat = vfs.stat(file_id).expect("Failed to stat file");
        assert_eq!(stat.size, data.len() as u64);

        let read_data = vfs.read(file_id).expect("Failed to read file");
        assert_eq!(read_data, data);
    }

    #[test]
    fn test_nested_directories() {
        let vfs = create_test_vfs();
        
        let dir1 = vfs.mkdir(1, "dir1").expect("Failed to create dir1");
        let dir2 = vfs.mkdir(dir1, "dir2").expect("Failed to create dir2");
        let file = vfs.create_file(dir2, "file.txt").expect("Failed to create file");

        vfs.write(file, b"nested content").expect("Failed to write file");

        // Test directory traversal
        let entries = vfs.read_dir(1).expect("Failed to read root");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "dir1");

        let entries = vfs.read_dir(dir1).expect("Failed to read dir1");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "dir2");

        let entries = vfs.read_dir(dir2).expect("Failed to read dir2");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "file.txt");

        let content = vfs.read(file).expect("Failed to read file");
        assert_eq!(content, b"nested content");
    }

    #[test]
    fn test_lookup() {
        let vfs = create_test_vfs();
        let dir_id = vfs.mkdir(1, "mydir").expect("Failed to create directory");
        
        let found_id = vfs.lookup(1, "mydir").expect("Failed to lookup directory");
        assert_eq!(found_id, dir_id);

        // Test non-existent lookup
        assert!(vfs.lookup(1, "nonexistent").is_err());
    }

    #[test]
    fn test_resolve_path() {
        let vfs = create_test_vfs();
        
        let dir1 = vfs.mkdir(1, "dir1").expect("Failed to create dir1");
        let dir2 = vfs.mkdir(dir1, "dir2").expect("Failed to create dir2");
        let file = vfs.create_file(dir2, "file.txt").expect("Failed to create file");

        // Test path resolution
        assert_eq!(vfs.resolve_path("/").unwrap(), 1);
        assert_eq!(vfs.resolve_path("").unwrap(), 1);
        assert_eq!(vfs.resolve_path("/dir1").unwrap(), dir1);
        assert_eq!(vfs.resolve_path("/dir1/dir2").unwrap(), dir2);
        assert_eq!(vfs.resolve_path("/dir1/dir2/file.txt").unwrap(), file);

        // Test non-existent path
        assert!(vfs.resolve_path("/nonexistent").is_err());
    }

    #[test]
    fn test_rename() {
        let vfs = create_test_vfs();
        let file_id = vfs.create_file(1, "oldname").expect("Failed to create file");
        vfs.write(file_id, b"test content").expect("Failed to write file");

        vfs.rename(1, "oldname", 1, "newname").expect("Failed to rename file");

        // Old name should not exist
        assert!(vfs.lookup(1, "oldname").is_err());
        
        // New name should exist and point to same inode
        let renamed_id = vfs.lookup(1, "newname").expect("Failed to lookup renamed file");
        assert_eq!(renamed_id, file_id);

        // Content should be preserved
        let content = vfs.read(file_id).expect("Failed to read file");
        assert_eq!(content, b"test content");
    }

    #[test]
    fn test_move_between_directories() {
        let vfs = create_test_vfs();
        let dir1 = vfs.mkdir(1, "dir1").expect("Failed to create dir1");
        let dir2 = vfs.mkdir(1, "dir2").expect("Failed to create dir2");
        let file_id = vfs.create_file(dir1, "moveme.txt").expect("Failed to create file");

        vfs.rename(dir1, "moveme.txt", dir2, "moved.txt").expect("Failed to move file");

        // File should no longer be in dir1
        let entries1 = vfs.read_dir(dir1).expect("Failed to read dir1");
        assert_eq!(entries1.len(), 0);

        // File should be in dir2
        let entries2 = vfs.read_dir(dir2).expect("Failed to read dir2");
        assert_eq!(entries2.len(), 1);
        assert_eq!(entries2[0].name, "moved.txt");
        assert_eq!(entries2[0].inode_id, file_id);
    }

    #[test]
    fn test_remove_file() {
        let vfs = create_test_vfs();
        let file_id = vfs.create_file(1, "deleteme.txt").expect("Failed to create file");
        vfs.write(file_id, b"content").expect("Failed to write file");

        vfs.remove_file(1, "deleteme.txt").expect("Failed to remove file");

        // File should no longer exist
        assert!(vfs.lookup(1, "deleteme.txt").is_err());
        assert!(vfs.stat(file_id).is_err());

        // Root should be empty
        let entries = vfs.read_dir(1).expect("Failed to read root");
        assert_eq!(entries.len(), 0);
    }

    #[test]
    fn test_remove_empty_directory() {
        let vfs = create_test_vfs();
        let dir_id = vfs.mkdir(1, "emptydir").expect("Failed to create directory");

        vfs.remove_dir(1, "emptydir").expect("Failed to remove directory");

        // Directory should no longer exist
        assert!(vfs.lookup(1, "emptydir").is_err());
        assert!(vfs.stat(dir_id).is_err());
    }

    #[test]
    fn test_remove_non_empty_directory_fails() {
        let vfs = create_test_vfs();
        let dir_id = vfs.mkdir(1, "nonemptydir").expect("Failed to create directory");
        vfs.create_file(dir_id, "file.txt").expect("Failed to create file");

        let result = vfs.remove_dir(1, "nonemptydir");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CraftVfsError::DirectoryNotEmpty));
    }

    #[test]
    fn test_file_already_exists_error() {
        let vfs = create_test_vfs();
        vfs.create_file(1, "duplicate").expect("Failed to create first file");
        
        let result = vfs.create_file(1, "duplicate");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CraftVfsError::FileExists(_)));
    }

    #[test]
    fn test_directory_already_exists_error() {
        let vfs = create_test_vfs();
        vfs.mkdir(1, "duplicate").expect("Failed to create first directory");
        
        let result = vfs.mkdir(1, "duplicate");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CraftVfsError::FileExists(_)));
    }

    #[test]
    fn test_write_to_directory_fails() {
        let vfs = create_test_vfs();
        let dir_id = vfs.mkdir(1, "testdir").expect("Failed to create directory");
        
        let result = vfs.write(dir_id, b"data");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CraftVfsError::NotAFile));
    }

    #[test]
    fn test_read_directory_as_file_fails() {
        let vfs = create_test_vfs();
        let dir_id = vfs.mkdir(1, "testdir").expect("Failed to create directory");
        
        let result = vfs.read(dir_id);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CraftVfsError::NotAFile));
    }

    #[test]
    fn test_read_dir_on_file_fails() {
        let vfs = create_test_vfs();
        let file_id = vfs.create_file(1, "testfile").expect("Failed to create file");
        
        let result = vfs.read_dir(file_id);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CraftVfsError::NotADirectory));
    }
}