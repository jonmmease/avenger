---
description: Display the contents of a guide
---

# Read Guide

Display the contents of a guide from the `guides/` directory.

## Guide ID

{{#if args}}
**Requested:** {{args}}
{{else}}
No guide ID provided. Use `/guide-list` to see available guides, then run `/guide-read [id]`.
{{/if}}

## Workflow

### Step 1: Find the Guide
1. List files in `guides/` directory
2. Look for exact match: `guides/{{args}}.md`
3. If no exact match, search for guides containing the term

### Step 2: Handle Fuzzy Matching
If no exact match found:
1. Find all guide filenames containing the search term
2. If multiple matches, use AskUserQuestion to let user choose:
   ```
   "Which guide did you mean?"
   - option 1: guide-name-a
   - option 2: guide-name-b
   ```
3. If no matches, tell user the guide wasn't found and suggest `/guide-list`

### Step 3: Display Content
Once the guide is identified:
1. Read the guide file
2. Display its full contents

## Examples

- `/guide-read architecture-patterns` → exact match
- `/guide-read facet` → fuzzy match, may prompt for clarification
- `/guide-read xyz` → not found, suggest alternatives
