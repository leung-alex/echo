"""Compare timestamped desktop pixels with each trial's settled card regions."""
import argparse,gzip,json,pathlib,math
import numpy as np
from PIL import Image,ImageDraw,ImageFilter

def analyze(root, row):
    name=row['name'];meta=json.loads((root/(name+'.frames.json')).read_text())
    events=json.loads((root/(name+'.trace.json')).read_text())['events']
    layout=[e['detail'] for e in events if e['event']=='layout_ready'][-1]
    frames=np.frombuffer(gzip.decompress((root/(name+'.bgra.gz')).read_bytes()),dtype=np.uint8).reshape(-1,meta['height'],meta['width'],4)[:,:,:,:3][:,:,:,::-1]
    factor=meta['factor'];origin=meta['origin'];h,w=frames.shape[1:3]
    def point(x,y):return ((x-origin[0])/factor,(y-origin[1])/factor)
    x,y,cw,ch=layout['card']
    def rect_mask(a,b):
        mask=Image.new('L',(w,h));ImageDraw.Draw(mask).rectangle([point(*a),point(*b)],fill=255);return np.array(mask)>0
    main=rect_mask((x+24,y+12),(x+cw-24,y+64))
    side_image=Image.new('L',(w,h));quad=layout['side_quad']
    if not quad:raise RuntimeError(f'No side geometry: {name}')
    for px,py in [point(x,y),point(x+cw,y+ch),*[point(*p) for p in quad]]:
        if not (0<=px<w and 0<=py<h):
            raise RuntimeError(f'Capture omitted part of the popup: {name} {(px,py)} outside {(w,h)}')
    ImageDraw.Draw(side_image).polygon([point(*p) for p in quad],fill=255)
    side=np.array(side_image.filter(ImageFilter.MinFilter(3)))>0
    side &= ~rect_mask((x-4,y-4),(x+cw+4,y+ch+4))
    reference=np.median(frames[-5:],axis=0).astype(np.int16)
    baseline=frames[0].astype(np.int16)
    changed=np.max(np.abs(reference-baseline),axis=-1)>24
    # Require rendered body detail as well as the header. A blank card is not a
    # faster completed popup. Ignore flat fill pixels which could hide missing text.
    detail=(np.max(np.abs(reference-np.roll(reference,1,axis=0)),axis=-1)>20)|(np.max(np.abs(reference-np.roll(reference,1,axis=1)),axis=-1)>20)
    body=rect_mask((x+24,y+78),(x+cw-24,y+ch-28))&detail
    times=np.array(meta['frames'])
    matches=np.stack([np.max(np.abs(frame.astype(np.int16)-reference),axis=-1)<=20 for frame in frames])
    def detect(mask,threshold):
        pixels=mask&changed
        if pixels.sum()<40:raise RuntimeError(f'Insufficient changed pixels: {name} {pixels.sum()}')
        scores=matches[:,pixels].mean(axis=1).tolist()
        for i in range(1,len(scores)-2):
            if min(scores[i:i+3])>=threshold:
                return dict(observed_ms=float(times[i,1]),lower_ms=float(max(0,times[i-1,0])),
                    upper_ms=float(times[i,1]),pixels=int(pixels.sum()),frame=i,scores=scores)
        raise RuntimeError(f'No stable appearance: {name}')
    first=detect(main,.75);full=detect(side,.98);body_ready=detect(body,.98);side_detail=detect(side&detail,.98)
    complete=dict(lower_ms=max(first['lower_ms'],full['lower_ms'],body_ready['lower_ms'],side_detail['lower_ms']),
        upper_ms=max(first['upper_ms'],full['upper_ms'],body_ready['upper_ms'],side_detail['upper_ms']))
    stages={}
    for event in events:stages.setdefault(event['event'],event['us']/1000)
    intervals=np.diff(times[1:,1]);capture=times[1:,1]-times[1:,0]
    result=dict(name=name,target=row['target'],kind=row['kind'],side='right' if layout['right'] else 'left',
        main=first,main_content=body_ready,side_complete=full,side_content=side_detail,complete=complete,stages_ms=stages,
        hidden_wait_ms=stages.get('uncloaked',0)-stages.get('first_frame_hidden',0),
        fps=1000/float(np.mean(intervals)),interval_p95_ms=float(np.percentile(intervals,95)),
        capture_median_ms=float(np.median(capture)))
    bounds=[(x,y),(x+cw,y+ch),*quad];xs,ys=zip(*[point(*p) for p in bounds])
    crop=(max(0,int(min(xs))-6),max(0,int(min(ys))-6),min(w,int(max(xs))+7),min(h,int(max(ys))+7))
    Image.fromarray(frames[-1]).crop(crop).save(root/(name+'.settled.png'))
    Image.fromarray(frames[first['frame']]).crop(crop).save(root/(name+'.first-visible.png'))
    (root/(name+'.analysis.json')).write_text(json.dumps(result,indent=2),encoding='utf-8')
    return result

def main():
    parser=argparse.ArgumentParser();parser.add_argument('roots',nargs='+',type=pathlib.Path);args=parser.parse_args()
    all_results=[]
    for root in args.roots:
        for row in json.loads((root/'runs.json').read_text()):all_results.append(analyze(root,row))
    groups=[]
    for target in sorted({r['target'] for r in all_results}):
        for kind in ['first','repeat']:
            for side in ['all','left','right']:
                rows=[r for r in all_results if r['target']==target and r['kind']==kind and (side=='all' or r['side']==side)]
                if not rows:continue
                stats={}
                for label,values in [('complete_ms',[r['complete']['upper_ms'] for r in rows]),('main_ms',[r['main']['observed_ms'] for r in rows]),('side_ms',[r['side_complete']['observed_ms'] for r in rows]),('hidden_wait_ms',[r['hidden_wait_ms'] for r in rows])]:
                    stats[label]=dict(median=float(np.median(values)),p95=float(np.percentile(values,95,method='higher')),maximum=max(values))
                stats['main_median_interval_ms']=[float(np.median([r['main']['lower_ms'] for r in rows])),float(np.median([r['main']['upper_ms'] for r in rows]))]
                stats['side_median_interval_ms']=[float(np.median([r['side_complete']['lower_ms'] for r in rows])),float(np.median([r['side_complete']['upper_ms'] for r in rows]))]
                stages={key:float(np.median([r['stages_ms'][key] for r in rows if key in r['stages_ms']])) for key in {key for r in rows for key in r['stages_ms']}}
                groups.append(dict(target=target,kind=kind,side=side,n=len(rows),stats=stats,median_stages_ms=stages))
    output=args.roots[0].parent/'summary.json'
    output.write_text(json.dumps(dict(groups=groups,samples=all_results),indent=2),encoding='utf-8')
    print(json.dumps(groups,indent=2))

if __name__=='__main__':main()
