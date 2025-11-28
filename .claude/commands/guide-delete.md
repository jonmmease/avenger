---
description: Remove an outdated guide
---

# Delete Guide

Remove a guide from the `guides/` directory.

## Guide ID

{{#if args}}
**Requested:** {{args}}
{{else}}
No guide ID provided. Use `/guide-list` to see available guides, then run `/guide-delete [id]`.
{{/if}}

## Workflow

### Step 1: Find the Guide
1. List files in `guides/` directory
2. Look for exact match: `guides/{{args}}.md`
3. If no exact match, search for guides containing the term

### Step 2: Handle Fuzzy Matching
If no exact match found:
1. Find all guide filenames containing the search term
2. Use AskUserQuestion to let user choose:
   ```
   "Which guide did you want to delete?"
   - option 1: guide-name-a
   - option 2: guide-name-b
   ```
3. If no matches, tell user the guide wasn't found and suggest `/guide-list`

### Step 3: Confirm Deletion
**Always confirm before deleting.** Use AskUserQuestion:
```
"Are you sure you want to delete 'guide-name.md'? This cannot be undone."
- Yes, delete it
- No, cancel
```

### Step 4: Delete
If confirmed:
1. Delete the guide file from `guides/`
2. Remove the corresponding row from `guides/INDEX.md`
3. Confirm deletion to the user

If cancelled:
- Tell user the deletion was cancelled

## Important Notes
- INDEX.md itself should never be deleted
- Deletion is permanent (though recoverable via git if not committed)
- Consider if the guide should be updated rather than deleted
