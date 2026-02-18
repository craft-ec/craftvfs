# CraftVFS

CraftVFS is a distributed filesystem library built on top of CraftSQL. It provides POSIX-like filesystem operations backed by CraftSQL's content-addressable storage system.

## Features

- **Familiar API** - mkdir, create_file, read, write, read_dir operations
- **O(1) Renames** - Move files and directories without data copying  
- **Distributed Storage** - Backed by CraftSQL's content-addressable pages
- **Path Resolution** - Standard Unix-style path navigation
- **Comprehensive Testing** - Full test coverage for all operations

## Quick Start

```rust
use craftvfs::{CraftVfs, FileType};
use craftsql_store_local::LocalPageStore;

// Create a filesystem backed by local storage
let store = LocalPageStore::new(&std::path::Path::new("/tmp/my-vfs"))?;
let vfs = CraftVfs::open(store)?;

// Create a directory
let docs_dir = vfs.mkdir(1, "documents")?; // 1 = root directory

// Create a file
let file_id = vfs.create_file(docs_dir, "hello.txt")?;

// Write content
vfs.write(file_id, b"Hello, CraftVFS!")?;

// Read it back
let content = vfs.read(file_id)?;
println!("Content: {}", String::from_utf8_lossy(&content));

// List directory
for entry in vfs.read_dir(1)? {
    println!("Found: {} ({})", entry.name, 
        match entry.file_type {
            FileType::Dir => "directory",
            FileType::File => "file",
            FileType::Symlink => "symlink",
        }
    );
}

// Resolve paths
let file_id = vfs.resolve_path("/documents/hello.txt")?;
let stat = vfs.stat(file_id)?;
println!("File size: {} bytes", stat.size);
```

## Schema

The filesystem uses a simple but effective SQLite schema:

- **inodes** - File and directory metadata
- **dirents** - Directory entries (name → inode mappings)  
- **file_data** - File content as BLOBs

Root directory is inode 1, created automatically.

## Storage Backend

CraftVFS is storage-backend agnostic through the CraftSQL PageStore interface:

- **Local** - `craftsql-store-local` for single-machine usage
- **Network** - Other PageStore implementations for distributed setups
- **Memory** - For testing and ephemeral storage

## Performance

- **Renames** are O(1) - just updates directory entry
- **Reads/Writes** stream directly through SQLite to CraftSQL pages
- **Deduplication** provided by CraftSQL's content-addressable storage
- **Snapshots** planned through CraftSQL integration

## License

MIT