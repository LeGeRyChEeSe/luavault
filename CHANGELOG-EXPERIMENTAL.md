# Experimental changelog

The source of the **experimental channel**'s notes: `scripts/publish-release.ps1 -Channel
experimental` reads it for the `notes` and `history` of `manifest-experimental.json`, exactly as
`CHANGELOG.md` for the stable. Same rules, same guards (a missing version or an entry without a
bullet fails the publication):

- a heading is written `## <x.y.z-exp.N> - <YYYY-MM-DD>` (ASCII dash, pre-release suffix);
- only the `- ` lines are kept, as plain text, one line per bullet;
- `### fr` and `### en` split the bullets per language.

An experimental version describes what it adds over the stable. A stable that takes a feature
over describes it again in `CHANGELOG.md`.

## Unreleased
