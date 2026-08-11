# packaging

Templates for the distribution channels; placeholders (`SHA_*`) are filled
with the sha256 of the release assets once the `release binaries` workflow
finishes, then pushed to their own repos:

- `homebrew/rxform.rb` + `TAP-README.md` → github.com/milkway/homebrew-tap (Formula/, README)
- `scoop/rxform.json` → github.com/milkway/scoop-bucket (bucket/)

Fill shas with: `curl -sL <asset-url> | shasum -a 256`.
