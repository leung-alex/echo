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
    # Bind retained content and memberships, not only IDs/timestamps. Exclude derived
    # preview caches and settings: resume state legitimately changes during a run.
    for table in ('clipboard_entries','clipboard_representations','clipboard_blobs',
                  'saved_items','saved_item_representations','tags','saved_item_tags',
                  'spaces','space_memberships'):
        columns=[r[1] for r in db.execute(f'PRAGMA table_info("{table}")')]
        if not columns:
            continue
        order=','.join('"'+c.replace('"','""')+'"' for c in columns)
        digest=hashlib.sha256()
        for row in db.execute(f'SELECT * FROM "{table}" ORDER BY {order}'):
            for value in row:
                encoded=(b'bytes:'+value) if isinstance(value,bytes) else json.dumps(value,ensure_ascii=False).encode()
                digest.update(len(encoded).to_bytes(8,'little'));digest.update(encoded)
        result.setdefault(table,{})['content_sha256']=digest.hexdigest()
    # Sign the retained original blob bytes as well as database references. A
    # stable row/hash field alone would miss corrupted or missing payload files.
    payloads=hashlib.sha256(); payload_count=0; payload_bytes=0
    for expected,size in db.execute('SELECT hash,byte_size FROM clipboard_blobs ORDER BY hash'):
        if len(expected)!=64 or any(c not in '0123456789abcdef' for c in expected):
            raise ValueError('Invalid retained blob identity')
        blob=path.parent/'blobs'/expected
        digest=hashlib.sha256(); actual_size=0
        with blob.open('rb') as stream:
            for chunk in iter(lambda:stream.read(1024*1024),b''):
                digest.update(chunk); actual_size+=len(chunk)
        if digest.hexdigest()!=expected or actual_size!=size:
            raise ValueError('Retained original blob bytes do not match their database identity')
        payloads.update(expected.encode()); payloads.update(actual_size.to_bytes(8,'little'))
        payload_count+=1; payload_bytes+=actual_size
    result['original_blob_files']={'count':payload_count,'bytes':payload_bytes,'verified_sha256':payloads.hexdigest()}
print(json.dumps(result,separators=(',',':')))
