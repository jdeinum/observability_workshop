# Sync changes to remote
sync:
    git add .
    git diff --cached --quiet || git commit -m "chore: syncing"
    git push
