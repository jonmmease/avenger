---
description: List all available guides with descriptions
---

# List Guides

Display all available guides from the `guides/` directory.

## Workflow

1. Read `guides/INDEX.md`
2. Parse the table to extract guide names and descriptions
3. Display them in a clean, readable format

## Output Format

Present the guides as:

```
Available Guides:

  project-overview          - Starting work on the project or need general orientation
  codebase-structure        - Need to understand project layout and module organization
  architecture-patterns     - Implementing new features or refactoring existing code
  ...
```

## Related Commands

- `/guide-read [id]` - Display a guide's contents
- `/guide-new [topic]` - Create a new guide
- `/guide-edit [id] [instructions]` - Modify an existing guide
- `/guide-delete [id]` - Remove a guide
