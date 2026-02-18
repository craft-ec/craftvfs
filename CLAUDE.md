# CLAUDE.md - CraftVFS Development Context

CraftVFS is a distributed filesystem library built on top of CraftSQL. It provides a POSIX-like filesystem interface backed by CraftSQL's content-addressable page storage.

## Architecture

### Core Components

- **CraftVfs** - Main filesystem interface
- **Database Schema** - SQLite tables for filesystem metadata
- **CraftSQL Integration** - Uses CraftSQL VFS for storage backend

### Schema Design

The filesystem uses three main tables:

1. **inodes** - File/directory metadata (size, type, timestamps, permissions)
2. **dirents** - Directory entries (parent-child relationships)
3. **file_data** - File content as BLOBs

Root directory is always inode ID 1, created automatically on initialization.

### Key Features

- **O(1) Rename/Move** - Just updates directory entry, no data copying
- **POSIX-like API** - mkdir, create_file, read, write, read_dir, etc.
- **Path Resolution** - resolve_path() walks directory hierarchy
- **Distributed Storage** - Backed by CraftSQL's content-addressable pages
- **Snapshots** - Planned integration with CraftSQL's snapshot capabilities

## Implementation Notes

### VFS Integration

- Registers CraftSQL VFS as "craftsql" 
- Opens SQLite connection using the custom VFS
- All data stored through CraftSQL's PageStore interface

### Error Handling

- Custom `CraftVfsError` enum with proper error context
- Validates file vs directory operations (no writing to dirs, etc.)
- Enforces directory emptiness for removal

### Testing

Comprehensive test suite covers:
- Basic file/directory operations
- Nested directory hierarchies  
- Read/write file content
- Rename and move operations
- Error cases (file exists, not empty, wrong type)
- Path resolution

## Dependencies

- `craftsql-core` - PageStore trait and core types
- `craftsql-vfs` - SQLite VFS integration  
- `craftsql-store-local` - For testing (dev dependency)
- `rusqlite` - SQLite interface
- `thiserror` - Error handling

## Development Status

- ✅ Core filesystem operations implemented
- ✅ Comprehensive test coverage
- ✅ Integration with CraftSQL VFS
- 🚧 Snapshots (placeholder - needs CraftSQL integration)
- 🚧 Symlinks (schema supports, not implemented)
- 🚧 Advanced permissions/ownership

## Performance Considerations

- Renames are O(1) operations
- Path resolution walks directory tree (could be optimized with caching)
- File content stored as BLOBs - efficient for small to medium files
- CraftSQL provides deduplication and content-addressable storage

## Usage Patterns

Designed for:
- Distributed applications needing filesystem semantics
- Content-addressable storage with filesystem interface
- Scenarios requiring snapshots and versioning
- Testing and development with familiar file operations

Not optimized for:
- Very large files (stored as single BLOBs)
- High-frequency small writes (SQLite transaction overhead)
- POSIX compliance edge cases