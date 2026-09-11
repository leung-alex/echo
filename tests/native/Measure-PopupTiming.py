"""Bounded DEV Alt+V measurement; owned synthetic inputs, no paste/submit."""
import argparse, datetime, gzip, json, os, pathlib, shutil, subprocess, time
import psutil

REPO = pathlib.Path(__file__).resolve().parents[2]
TITLE = 'Echo Recall'
NATIVE = 'Echo Popup Timing Native'
BROWSER = 'Echo Popup Timing Browser'

def atomic(path, data):
    tmp=path.with_suffix('.tmp');tmp.write_text(json.dumps(data),encoding='utf-8');tmp.replace(path)

def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('root',type=pathlib.Path)
    parser.add_argument('--target',choices=['native','browser'],required=True)
    parser.add_argument('--sampler',choices=['gdi','dxgi'],default='gdi')
    parser.add_argument('--probe',action='store_true')
    parser.add_argument('--no-pixels',action='store_true',help='Observer-overhead control; internal timestamps only')
    parser.add_argument('--first-count',type=int,default=10)
    parser.add_argument('--repeat-count',type=int,default=30)
    parser.add_argument('--profile',choices=['dev','release'],default='dev')
    parser.add_argument('--idle-seconds',type=float,default=0)
    parser.add_argument('--query-cycle',action='store_true',help='Native fixture alternates matching and empty result queries')
    parser.add_argument('--renderer',choices=['default','software'],default='default')
    parser.add_argument('--interaction-check',action='store_true')
    parser.add_argument('--keyboard-navigation',action='store_true',help='Use Shift+Tab during the optional interaction check')
    args=parser.parse_args();root=args.root.resolve()
    if args.first_count < 1 or args.repeat_count < 0:
        parser.error('At least one first activation is required to start the measured process')
    if (root/'runs.json').exists():
        raise RuntimeError('Use a fresh evidence root; completed samples must not be overwritten')
    marker=json.loads((root/'data/synthetic-fixture.json').read_text(encoding='utf-8-sig'))
    if marker.get('synthetic') is not True or marker.get('capture_enabled') is not False:
        raise RuntimeError('Only capture-disabled synthetic fixtures may be measured')
    env=dict(os.environ,ECHO_WINDOWS_ACCEPTANCE='1',ECHO_DATA_DIR=str(root/'data'),ECHO_POPUP_TIMING_DIR=str(root))
    if args.renderer=='software':env['ECHO_RENDERER']='software'
    if args.query_cycle and args.target!='native':raise ValueError('Query-cycle requires the owned native fixture')
    owned=[];processes=[];echo=None;recording=None;manager_bounds=None
    def launch(exe, argv):
        p=subprocess.Popen([str(exe),*map(str,argv)],env=env,stdin=subprocess.DEVNULL,
            stdout=open(root/(str(len(processes))+'-stdout.log'),'w'),stderr=open(root/(str(len(processes))+'-stderr.log'),'w'))
        processes.append(p);return p
    def register(p,title):
        actual=psutil.Process(p.pid)
        owned.append(dict(pid=p.pid,title=title,executable=actual.exe(),
            started_utc=datetime.datetime.fromtimestamp(actual.create_time(),datetime.timezone.utc).isoformat()))
        atomic(root/'owned-processes.json',owned)
    def tool(op,p=None,title=None,*argv):
        command=[str(root/'EchoInlineDriver.exe'),op,str(root)]
        command+= [str(p.pid),title,*map(str,argv)] if p else list(map(str,argv))
        r=subprocess.run(command,env=env,stdin=subprocess.DEVNULL,capture_output=True,text=True,encoding='utf-8',timeout=25,creationflags=subprocess.CREATE_NO_WINDOW)
        if r.returncode: raise RuntimeError(r.stdout+r.stderr)
        return json.loads(r.stdout.lstrip('\ufeff'))['value']
    def wait(fn,label,timeout=15):
        until=time.monotonic()+timeout;last=None
        while time.monotonic()<until:
            try:
                value=fn()
                if value:return value
            except Exception as e:last=e
            time.sleep(.1)
        raise RuntimeError(f'{label}: {last}')
    def native(op,**values):
        ident=str(time.monotonic_ns());atomic(root/'native-command.json',dict(id=ident,op=op,**values))
        def response():
            v=json.loads((root/'native-response.json').read_text(encoding='utf-8'))
            if v['id']==ident:
                if v['status']!='PASS':raise RuntimeError(v)
                return v
        return wait(response,op)['value']
    def stop_echo():
        nonlocal echo
        if echo and echo.poll() is None:
            subprocess.run([str(root/'echo-timing.exe'),'--quit'],env=env,timeout=10,creationflags=subprocess.CREATE_NO_WINDOW)
            echo.wait(10)
        echo=None
    def start_echo():
        nonlocal echo,manager_bounds
        stop_echo()
        wait(lambda:tool('probe',None,None,'Alt+V')['available'],'Alt+V available')
        echo=launch(root/'echo-timing.exe',['--history']);register(echo,TITLE)
        wait(lambda:tool('ready',echo,TITLE),'manager content ready')
        wait(lambda:not tool('probe',None,None,'Alt+V')['available'],'hotkey registration')
        if args.interaction_check:manager_bounds=tool('geometry',echo,TITLE)['window']
        tool('close-owned',echo,TITLE)
        time.sleep(.2)
    results=[];empty_query_height=None
    try:
        if args.target=='native':
            target=launch(root/'EchoInlineFixture.exe',[root,NATIVE]);title=NATIVE;register(target,title)
            wait(lambda:tool('geometry',target,title),'native window')
            control='Inline fixture single'
        else:
            html=root/'popup-composer.html'
            html.write_text('''<!doctype html><meta charset="utf-8"><title>Echo Popup Timing Browser</title>
<style>body{background:#334860;color:white;font:18px Segoe UI;margin:24px}textarea{display:block;margin-top:30px;width:650px;height:90px;font:18px Segoe UI}</style>
<h1>Isolated Alt+V timing</h1><textarea aria-label="Timing composer" autofocus></textarea>
<script>document.querySelector('textarea').addEventListener('keydown',e=>{if(e.key==='Enter')e.preventDefault()});</script>''',encoding='utf-8')
            target=launch(pathlib.Path(r'C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe'),[
                '--user-data-dir='+str(root/'browser-profile'),'--no-first-run','--no-default-browser-check',
                '--disable-background-networking','--disable-sync','--disable-extensions','--disable-background-mode',
                '--force-renderer-accessibility','--window-size=800,600','--window-position=60,300','--app='+html.as_uri()])
            title=BROWSER;register(target,title);control='Timing composer'
            wait(lambda:tool('geometry',target,title),'browser window')
        geometry=tool('geometry',target,title);work=geometry['work'];scale=geometry['dpi']/96
        atomic(root/'environment.json',dict(target=args.target,geometry=geometry,profile=args.profile,startup='manager shown and content-ready, then hidden before first Alt+V',browser_accessibility=args.target=='browser'))
        count=1 if args.probe else args.first_count
        for kind,total in [('first',count),('repeat',0 if args.probe else args.repeat_count)]:
            for index in range(total):
                if kind=='first':start_echo()
                # Re-establish the owned fixture after a deep-hide wait. Shell
                # notifications can change foreground during the 36-second gap;
                # the driver still rechecks foreground immediately before input.
                if args.idle_seconds and kind=='repeat':time.sleep(args.idle_seconds)
                side='right' if index%2==0 else 'left'
                x=work[0]+50 if side=='right' else work[2]-int(650*scale)
                y=work[1]+int(220*scale)
                tool('move',target,title,x,y)
                query=['','echo','','unmatched-popup-query','fixture',''][index%6] if args.query_cycle else ''
                initially_selected = args.query_cycle and query=='unmatched-popup-query'
                if args.target=='native':native('reset',control='single',text=query if initially_selected else '',start=0,length=len(query) if initially_selected else 0)
                tool('activate-owned',target,title,control)
                tool('english-owned',target,title)
                if initially_selected:
                    # activate-owned clicks the edit and collapses its selection.
                    # Restore and verify the intended selection after that click.
                    selection=native('reset',control='single',text=query,start=0,length=len(query))
                    field=selection['fields']['single']
                    assert field['start']==0 and field['length']==len(query)
                time.sleep(.2)
                # The owned fixture and both possible card projections fit this ROI.
                # Derive it before sending the hotkey; no UIA calls inside the timed capture.
                region=tool('geometry',target,title)
                caret=region.get('caret') or tool('card',target,title,control)
                cx,cy,rx,by=caret
                # The fixtures place the empty caret on a known side, with room below.
                # Keep full card/shadow coverage while avoiding a full-monitor readback.
                # The analyzer rejects every sample whose actual projected card escapes.
                left=cx-int((50 if side=='right' else 460)*scale)
                right=cx+int((980 if side=='right' else 570)*scale)
                atomic(root/'capture-region.json',[max(work[0],left),max(work[1],cy-int(70*scale)),min(work[2],right),min(work[3],by+int(620*scale))])
                name=f'{args.target}-{kind}-{index:02d}-{side}'
                before=set(root.glob('trace-*.json'))
                if args.query_cycle:
                    tool('hotkey',target,title,'Alt+V')
                    wait(lambda:tool('card',echo,TITLE,'History space'),'query popup')
                    time.sleep(.4)
                    initial_bounds=tool('card',echo,TITLE,'History space')
                    if query and not initially_selected:tool('text',target,title,query)
                    def query_ready():
                        value=tool('read-control',target,title,control)
                        tree=tool('dump',echo,TITLE)
                        if value!=query:return None
                        if query=='unmatched-popup-query':
                            return tree if 'No matches' in tree and 'ControlType.ListItem' not in tree else None
                        return tree if 'ControlType.ListItem' in tree else None
                    tree=wait(query_ready,'actual typed query results')
                    time.sleep(.4)
                    final_bounds=tool('card',echo,TITLE,'History space')
                    if not query:empty_query_height=final_bounds[3]-final_bounds[1]
                    if query=='unmatched-popup-query':assert empty_query_height is not None and final_bounds[3]-final_bounds[1]<empty_query_height
                    atomic(root/(name+'.query-uia.json'),tree)
                    screenshot=tool('screen-owned',echo,TITLE,name+'.query.png')
                    sample=dict(capture=False,functional_query=True,status='PASS',typed_query=query,initially_selected=initially_selected,initial_bounds=initial_bounds,final_bounds=final_bounds,screenshot=screenshot,note='Owned query typed after activation or selected before activation; not an Alt+V timing trial')
                elif args.no_pixels:
                    tool('hotkey',target,title,'Alt+V');time.sleep(3)
                    sample=dict(capture=False,note='internal timestamp observer-overhead control')
                elif args.sampler=='dxgi':
                    from PopupDesktopFrames import measure
                    sample=measure(root,name,lambda:tool('timed-hotkey',target,title,'Alt+V'))
                else:sample=tool('measure-open',target,title,name,3000)
                if index==total-1:
                    atomic(root/(name+'.uia.json'),tool('dump',echo,TITLE))
                tool('key',target,title,27)
                traces=wait(lambda:list(set(root.glob('trace-*.json'))-before),'trace flushed')
                trace=traces[0];saved=root/(name+'.trace.json');shutil.copyfile(trace,saved)
                row=dict(name=name,target=args.target,kind=kind,side=side,query=query,sample=sample,trace=str(saved))
                results.append(row);atomic(root/'runs.json',results)
                print(json.dumps(dict(completed=len(results),name=name)),flush=True)
                time.sleep(.25)
        if args.interaction_check:
            if args.target!='native' or args.renderer=='software':raise ValueError('Interaction scenario requires native GPU fixture')
            checks=[]
            native('reset',control='single',text='',start=0,length=0)
            tool('activate-owned',target,title,control)
            tool('hotkey',target,title,'Alt+V')
            wait(lambda:tool('card',echo,TITLE,'History space'),'popup available for interaction')
            time.sleep(.4)
            events=json.loads(saved.read_text())['events']
            quad=[e['detail']['side_quad'] for e in events if e['event']=='layout_ready'][-1]
            recording=subprocess.Popen([str(root/'EchoInlineDriver.exe'),'record-window',str(root),str(echo.pid),TITLE,str(target.pid),title,'navigation-frames'],env=env,stdout=open(root/'navigation-recording.log','w'),stderr=subprocess.STDOUT,creationflags=subprocess.CREATE_NO_WINDOW)
            time.sleep(.2)
            checks.append(tool('click-point',echo,TITLE,round(sum(p[0] for p in quad)/4),round(sum(p[1] for p in quad)/4)))
            wait(lambda:tool('card',echo,TITLE,'Favorites space'),'Favorites after side-card click')
            time.sleep(.4)
            checks.append(tool('screen-owned',echo,TITLE,'favorites-after-click.png'))
            if args.keyboard_navigation:
                checks.append(tool('paced-hotkey',target,title,'Shift+Tab',20))
            else:
                checks.append(tool('click-owned',echo,TITLE,'Previous space'))
            wait(lambda:tool('card',echo,TITLE,'History space'),'History after navigation')
            time.sleep(.4)
            checks.append(tool('screen-owned',echo,TITLE,'history-after-navigation.png'))
            (root/'navigation-frames'/'stop').touch()
            recording.wait(10)
            # This verifies native hit testing reaches the underlying owned input.
            bounds=tool('card',target,title,control)
            # The middle of this wide fixture edit lies under the card's shadow.
            # Its right end is inside the popup canvas but outside both card regions.
            checks.append(tool('click-point',target,title,round(bounds[2]-12),round((bounds[1]+bounds[3])/2)))
            tool('key',target,title,27)
            atomic(root/'interaction-checks.json',dict(status='PASS',checks=checks,note='Synthetic clicks/keys, not physical keyboard acceptance'))
            subprocess.run([str(root/'echo-timing.exe'),'--history'],env=env,timeout=10,check=True,creationflags=subprocess.CREATE_NO_WINDOW)
            wait(lambda:tool('ready',echo,TITLE),'manager after popup')
            time.sleep(.2)
            restored=tool('geometry',echo,TITLE)['window']
            if restored!=manager_bounds:raise RuntimeError(f'Manager geometry changed: {manager_bounds} -> {restored}')
            screenshot=tool('screen-owned',echo,TITLE,'manager-after-popup.png')
            atomic(root/'manager-roundtrip.json',dict(status='PASS',before=manager_bounds,after=restored,screenshot=screenshot))
    finally:
        if recording and recording.poll() is None:
            (root/'navigation-frames').mkdir(exist_ok=True)
            (root/'navigation-frames'/'stop').touch()
            recording.wait(10)
        stop_echo()
        for p in processes:
            if p.poll() is None:
                if args.target=='native' and p.pid==target.pid:
                    try:native('quit')
                    except Exception:pass
                elif args.target=='browser' and p.pid==target.pid:
                    try:tool('close-owned',target,title)
                    except Exception:pass
        atomic(root/'owned-processes.json',owned)

if __name__=='__main__': main()
