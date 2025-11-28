---
description: Create a new guide by exploring the codebase
---

# Create New Guide

You are creating a new guide for the project's `guides/` directory.

## Topic

{{#if args}}
**Topic:** {{args}}
{{else}}
No topic provided. Ask the user what topic they want a guide for.
{{/if}}

## Workflow

### Step 1: Derive Guide Name
Convert the topic to kebab-case for the filename (e.g., "event handling" → `event-handling.md`).

### Step 2: Explore the Codebase
Use the Task tool with `subagent_type=Explore` to thoroughly research the topic:
- Find relevant source files, modules, and patterns
- Understand how the feature/concept works
- Identify key types, functions, and their relationships
- Look for existing documentation or comments

### Step 3: Write the Guide
Create `guides/[topic-in-kebab-case].md` with:

1. **Title and Overview**: What this guide covers
2. **Key Concepts**: Core ideas and terminology
3. **Architecture/Design**: How components fit together
4. **Code References**: Important files and line numbers
5. **Common Patterns**: How to use/extend the feature
6. **Gotchas/Notes**: Non-obvious behaviors or considerations

Use code blocks with file paths when referencing specific code.

### Step 4: Update INDEX.md
Add a new row to the table in `guides/INDEX.md`:
```markdown
| `guides/[filename].md` | [When to read this guide] |
```

### Step 5: Confirm
Tell the user the guide was created and show the INDEX.md entry.

## Important Notes
- Keep guides focused and practical
- Include specific file paths and line numbers where helpful
- Write for someone unfamiliar with the codebase
- Update existing guides if the topic overlaps significantly
