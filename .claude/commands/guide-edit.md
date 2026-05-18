---
description: Modify an existing guide
---

# Edit Guide

Modify an existing guide in the `guides/` directory.

## Arguments

{{#if args}}
**Input:** {{args}}

Parse the input to extract:
- **Guide ID**: First word/term (the guide to edit)
- **Instructions**: Everything after the guide ID (what changes to make)
{{else}}
No arguments provided. Usage: `/guide-edit [guide-id] [instructions]`

Example: `/guide-edit architecture Add a section about the new plugin system`
{{/if}}

## Workflow

### Step 1: Find the Guide
1. List files in `guides/` directory
2. Look for exact match: `guides/[guide-id].md`
3. If no exact match, search for guides containing the term

### Step 2: Handle Fuzzy Matching
If no exact match found:
1. Find all guide filenames containing the search term
2. Use AskUserQuestion to let user confirm/choose:
   ```
   "Which guide did you mean?"
   - option 1: facet-layout-overflow-model
   - option 2: facet-invariants
   - option 3: overflow-measurement-analysis
   ```
3. If no matches, tell user the guide wasn't found and suggest `/guide-list`

### Step 3: Read Current Content
Read the guide file to understand its current structure and content.

### Step 4: Apply Changes
Based on the instructions:
- Add new sections
- Update existing content
- Remove outdated information
- Restructure as needed

Preserve the overall guide style and format.

### Step 5: Update INDEX.md (if needed)
If the guide's purpose/description changed significantly, update its entry in `guides/INDEX.md`.

### Step 6: Confirm
Show a summary of changes made.

## Examples

- `/guide-edit architecture Add a section about error handling patterns`
- `/guide-edit facet Update the overflow model diagram`
- `/guide-edit codebase Remove references to deprecated modules`
