"""Read-only logical signature; never exports clipboard contents."""
import hashlib, json, pathlib, sqlite3, sys
path=pathlib.Path(sys.argv[1]).resolve()
with sqlite3.connect(path.as_uri()+'?mode=ro',uri=True) as db:
    result={}
    for table in ('clipboard_entries','saved_items'):
        digest=hashlib.sha256(); count=0; maximum=0
        for row in db.execute(f'SELECT id, updated_at FROM {table} ORDER BY id'):
            digest.update(json.dumps(row,separators=(',',':')).encode()); digest.update(b'\n')
            count+=1; maximum=max(maximum,row[0])
        result[table]={'rows':count,'max_id':maximum,'identity_timestamp_sha256':digest.hexdigest()}
print(json.dumps(result,separators=(',',':')))
