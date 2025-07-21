---
allowed-tools: Bash(git add:*), Bash(git status:*)
description: Prepare changes for commit
---

## Context

- Current git status: !`git status`
- Current git diff (staged and unstaged changes): !`git diff HEAD`
- Current branch: !`git branch --show-current`
- Recent commits: !`git log --oneline -10`

## Your task

Review and improve the uncommitted changes. Do not make changes to code that was not modified since the last commit.

 - Review NEW comments for accuracy. Make sure they refer to the final state of the code, and not to steps along the way
 - Remove NEW dead code, include unused functions, unused function arguments, and unused variables
 - Run NEW tests
 - Perform formatting and fix lints
 - Perform type checking
 - Rerun formatting after fixing type check errors and make sure there are no errors
 - `git add` changed files
 - Draft and display a commit message. Do not attempt to commit changes yourself
