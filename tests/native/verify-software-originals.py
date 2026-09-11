"""Verify retained originals after the software gate's three favorite mutations."""
import json
import pathlib
import sqlite3
import sys

template, evidence = map(pathlib.Path, sys.argv[1:3])
before = json.loads((evidence / 'original-before.json').read_text(encoding='utf-8-sig'))
after = json.loads((evidence / 'original-after.json').read_text(encoding='utf-8-sig'))
# Only the Favorites space's revision and modification time may change. Keep the
# original full signatures as evidence; never weaken the shared read-only gate.
before.pop('spaces')
after.pop('spaces')
assert before == after, 'Retained originals or memberships changed'

def spaces(path):
    with sqlite3.connect(path.resolve().as_uri() + '?mode=ro', uri=True) as db:
        db.row_factory = sqlite3.Row
        return [dict(row) for row in db.execute('SELECT * FROM spaces ORDER BY id')]

old = spaces(template / 'echo.sqlite3')
new = spaces(evidence / 'data/echo.sqlite3')
assert len(old) == len(new), 'Space inventory changed'
favorites = 0
for original, current in zip(old, new):
    if original['kind'] == 'favorites':
        favorites += 1
        assert current['revision'] == original['revision'] + 3, 'Expected create/edit/delete revisions'
        assert current['updated_at'] >= original['updated_at'], 'Space timestamp regressed'
        current['revision'] = original['revision']
        current['updated_at'] = original['updated_at']
    assert original == current, 'Unrelated space metadata changed'
assert favorites == 1, 'Expected one Favorites space'
print(json.dumps({'status': 'PASS', 'favorite_mutations': 3, 'retained_originals': 'unchanged'}))
