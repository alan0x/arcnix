"""Session-only resource history and actual SVG interactions in the owned desktop."""
import json,time
from desktop_fixture import Peer,Desktop,expect,eventually,ROOT

out=ROOT/'artifacts/resource-trends-ui'/str(time.time_ns());checks=[]
def passed(value):checks.append(value);print(value,flush=True)
def data(p):return json.loads(p.locator('.resource-trend-data').text_content())
def queries(peer):
    path=peer.root/'metrics-queries.jsonl'
    return [json.loads(line) for line in path.read_text(encoding='utf-8').splitlines()] if path.exists() else []
def fixture(peer,**values):(peer.root/'metrics-fixture.json').write_text(json.dumps(dict(trace=True,**values)),encoding='utf-8')
def resources(app):app.application('Monitor')
def refresh(p):
    count=len(data(p)['points']);p.get_by_role('button',name='Refresh resource values',exact=True).click()
    eventually(lambda:len(data(p)['points'])==count+1)
    expect(p.get_by_role('button',name='Refresh resource values',exact=True)).to_be_enabled()

with Peer(out/'peer') as peer:
    fixture(peer)
    config=peer.config([peer.profile('Alpha'),peer.profile('Beta')],out)
    config['config']['settings']['resource_interval_seconds']=30
    with Desktop(out/'desktop',config) as app:
        p=app.page;app.connect();resources(app)
        expect(p.locator('.resource-trend-chart svg')).to_have_count(3)
        eventually(lambda:len(data(p)['points'])==1)
        assert data(p)['range']==300
        expect(p.locator('.resource-trend-windows .active')).to_have_text('5 min')
        first=data(p)['points'][0];assert first['cpu'] is None and first['receive'] is None and first['memory']==50
        expect(p.locator('.resource-trend-cpu')).to_contain_text('Unavailable')
        expect(p.locator('.resource-trend-memory .resource-trend-marker')).to_have_count(1)
        refresh(p);refresh(p)
        points=data(p)['points'];assert points[-1]['cpu']==30 and points[-1]['receive']==1024 and points[-1]['transmit']==2048
        assert points[0]['at']<points[1]['at']<points[2]['at']
        expect(p.locator('.resource-trend-cpu path.resource-trend-line')).to_have_count(1)
        assert p.locator('.resource-trend-chart svg').evaluate_all('els=>els.every(el=>el.getBoundingClientRect().width/el.viewBox.baseVal.width>=0.99)')
        expect(p.locator('.resource-trend-cpu .resource-trend-axis')).to_contain_text(['0%','25%','50%','75%','100%'])
        passed('Actual SSH samples retain first-read unknown rates, exact CPU/network deltas and monotonic client spacing; percentage axes stay fixed at 0–100%')

        memory_line=p.locator('.resource-trend-memory path.resource-trend-line')
        assert 'C' in memory_line.get_attribute('d')
        expect(p.locator('.resource-trend-memory path.resource-trend-area')).to_have_count(1)
        expect(p.locator('.resource-trend-network path.resource-trend-line')).to_have_count(2)
        p.emulate_media(reduced_motion='no-preference')
        # Give the next manual read a normal interval. Shorter intervals also
        # shorten the reveal so that a second read can take over promptly.
        time.sleep(1.8)
        refresh(p)
        memory=p.locator('.resource-trend-memory')
        tip=memory.locator('.resource-trend-tip')
        expect(tip).to_have_count(1)
        assert tip.evaluate("""async head => {
            const svg = head.closest('svg');
            const clip = svg.querySelector('clipPath rect');
            const line = svg.querySelector('path.resource-trend-line');
            const read = () => {
                const transform = line.parentNode.transform.baseVal;
                const anchor = transform.getItem(0).matrix.e + .5 * transform.getItem(1).matrix.a;
                const tip = head.transform.baseVal.getItem(0).matrix.e;
                return {anchor, tip, edge: clip.x.baseVal.value + clip.width.baseVal.value};
            };
            const first = read();
            await new Promise(resolve => setTimeout(resolve, 160));
            const next = read();
            return next.anchor < first.anchor && next.tip-next.anchor > first.tip-first.anchor
                && Math.abs(next.edge - next.tip) < .01;
        }""")
        p.emulate_media(reduced_motion='reduce')
        expect(tip).to_have_count(0)
        assert memory.locator('.resource-trend-series').get_attribute('clip-path') is None
        refresh(p)
        expect(p.locator('.resource-trend-live-tips')).to_have_count(0)
        p.emulate_media(reduced_motion='no-preference')
        passed('History scrolls left as the live tip draws the new segment on the same frames; reduced motion settles immediately')

        svg=p.locator('.resource-trend-cpu svg');p.locator('.resource-trend-toolbar button').last.focus();p.keyboard.press('Tab');expect(svg).to_be_focused()
        expect(svg).to_have_css('outline-style','solid')
        svg.press('Home')
        expect(p.locator('.resource-trend-cpu .resource-trend-tooltip')).to_contain_text('Unavailable')
        svg.press('End');expect(p.locator('.resource-trend-cpu .resource-trend-tooltip')).to_contain_text('30%')
        assert p.locator('.resource-trend-cpu .resource-trend-tooltip').evaluate('el=>el.style.left && el.getBoundingClientRect().right <= el.parentElement.getBoundingClientRect().right')
        svg.press('ArrowLeft');expect(svg).to_have_attribute('aria-label',__import__('re').compile('CPU usage'))
        svg.press('Escape');expect(p.locator('.resource-trend-cpu .resource-trend-tooltip')).not_to_be_visible()
        svg.hover(position=dict(x=130,y=90));expect(p.locator('.resource-trend-cpu .resource-trend-tooltip')).to_be_visible()
        memory_svg=p.locator('.resource-trend-memory svg')
        memory_svg.click(position=dict(x=130,y=90))
        expect(memory_svg).to_be_focused()
        expect(memory_svg).to_have_css('outline-style','none')
        expect(p.locator('.resource-trend-memory .resource-trend-tooltip')).to_be_visible()
        tip=p.get_by_role('button',name='Help: Session trends',exact=True);tip.hover();expect(p.get_by_role('tooltip')).to_contain_text('Gaps indicate missing data.');tip.click();p.mouse.move(1400,950);expect(p.get_by_role('tooltip')).to_be_visible();p.keyboard.press('Escape')
        passed('Charts expose samples by pointer and keyboard, outline only keyboard focus, and keep detailed guidance in hover/click help')

        sidebar=p.locator('.monitor-sidebar')
        for kind in ['cpu','memory','network']:
            sidebar.locator('.monitor-'+kind).click()
            expect(p.locator('.resource-trend-chart')).to_have_count(1)
            expect(p.locator('.resource-trend-'+kind+' svg')).to_be_visible()
            expect(p.locator('.resource-trend-'+kind+' path.resource-trend-line')).to_have_count(2 if kind=='network' else 1)
        sidebar.locator('.monitor-overview').click()
        expect(p.locator('.resource-trend-chart')).to_have_count(3)
        passed('CPU, memory and network detail pages each display their own trend and return to the three-chart overview')

        fixture(peer,error=True);refresh(p);assert data(p)['points'][-1]['memory'] is None
        expect(p.locator('.monitor-page .software-error')).to_be_visible()
        fixture(peer);refresh(p);assert data(p)['points'][-1]['cpu'] is None;refresh(p)
        expect(p.locator('.resource-trend-memory path.resource-trend-line')).to_have_count(2)
        fixture(peer,missing_memory=True,missing_network=True);refresh(p);point=data(p)['points'][-1]
        assert point['memory'] is None and point['receive'] is None and point['cpu']==30
        fixture(peer);refresh(p);refresh(p)
        fixture(peer,reset=True);refresh(p);point=data(p)['points'][-1]
        assert point['break_before'] and point['cpu'] is None and point['receive'] is None
        fixture(peer);refresh(p);refresh(p)
        passed('Read failures, missing metric sections and rebooted counters produce separate gaps and recover without turning unknown data into zero or connecting across a reboot')

        count=len(queries(peer));history=data(p)['points']
        for seconds,label in [(900,'15 min'),(60,'1 min'),(300,'5 min'),(60,'1 min')]:
            p.get_by_role('button',name=label,exact=True).click()
            eventually(lambda:data(p)['range']==seconds)
            expect(p.locator('.resource-trend-windows .active')).to_have_text(label)
            assert data(p)['points']==[point for point in history if history[-1]['at']-point['at']<=seconds]
        assert len(queries(peer))==count
        p.get_by_role('button',name='Clear history',exact=True).click();expect(p.locator('.resource-trend-data')).to_contain_text('"points":[]');assert len(queries(peer))==count
        refresh(p);assert len(data(p)['points'])==1 and data(p)['points'][0]['cpu'] is None
        refresh(p);refresh(p)
        passed('Range changes only select local history; clearing sends no command and the next read starts a fresh series')

        app.settings('Monitoring');p.locator('select.setting-resource-refresh-interval:visible').select_option(label='Every 3 seconds');app.servers();resources(app)
        expect(p.get_by_role('button',name='Refresh resource values',exact=True)).to_be_enabled()
        p.get_by_role('button',name='Pause sampling',exact=True).click();count=len(queries(peer));time.sleep(3.6);assert len(queries(peer))==count
        refresh(p);assert len(queries(peer))==count+1
        refresh(p);assert len(queries(peer))==count+2 and data(p)['points'][-1]['cpu']==30 and data(p)['points'][-1]['receive']==1024
        p.locator('.rail-scripts').click();count=len(queries(peer));time.sleep(3.6);assert len(queries(peer))==count
        app.servers();resources(app);assert len(queries(peer))==count
        p.get_by_role('button',name='Resume sampling',exact=True).click();eventually(lambda:len(queries(peer))>=count+2,timeout=9)
        expect(p.get_by_role('button',name='Refresh resource values',exact=True)).to_be_enabled()
        fixture(peer,delay=7);count=len(queries(peer));refresh(p);point=data(p)['points'][-1]
        assert point['break_before'] and point['cpu'] is None and point['receive'] is None,'A delayed read must not bridge a missing sampling interval'
        assert len(queries(peer))==count+1,'A pending read must not accumulate more sampling requests'
        fixture(peer)
        before=len(data(p)['points']);p.locator('.rail-scripts').click();count=len(queries(peer));time.sleep(3.6);assert len(queries(peer))==count
        app.servers();eventually(lambda:len(data(p)['points'])>before)
        assert data(p)['points'][before]['break_before']
        p.get_by_role('button',name='Pause sampling',exact=True).click()
        passed('Paused and hidden pages send no periodic requests; explicit refresh still works, and resuming restores automatic sampling')

        app.settings('General');p.locator('.setting-theme:visible').get_by_role('radio', name='Dark', exact=True).check();app.servers();resources(app)
        app.screenshot('resources-dark.png')
        dock=p.locator('.desktop-dock')
        app.settings('General');p.locator('.setting-theme:visible').get_by_role('radio', name='Light', exact=True).check();p.locator('.setting-language:visible').get_by_role('radio', name='简体中文', exact=True).check();app.servers();dock.get_by_role('button',name='监控',exact=True).click()
        p.locator('.monitor-sidebar').get_by_role('button',name='概览',exact=True).click()
        expect(p.locator('.resource-trend-title')).to_have_text(['CPU 使用率','内存使用','网络速率'])
        p.set_viewport_size(dict(width=900,height=780));expect(p.get_by_role('button',name='继续采样',exact=True)).to_be_visible()
        assert p.evaluate('document.documentElement.scrollWidth<=innerWidth')
        assert not p.locator('.resource-trends *:visible').evaluate_all("els=>els.filter(el=>!el.children.length&&el.textContent.trim()&&parseFloat(getComputedStyle(el).fontSize)<12).map(el=>el.textContent)")
        app.screenshot('resources-chinese-narrow.png')
        dock.get_by_role('button',name='文件管理',exact=True).click();dock.get_by_role('button',name='监控',exact=True).click()
        expect(p.locator('.resource-trend-chart svg')).to_have_count(3)
        passed('Chinese labels, both themes, 900px layouts and remounting preserve readable charts without duplicated renderers')

        p.set_viewport_size(dict(width=1440,height=980));p.locator('.settings-link').click();p.locator('.context-links').get_by_role('button',name='常规',exact=True).click();p.locator('.setting-language:visible').get_by_role('radio', name='English', exact=True).check();app.servers();app.connect(1);resources(app)
        eventually(lambda:len(data(p)['points'])==1)
        assert data(p)['points'][0]['cpu'] is None and len(data(p)['points'])==1
        saved=json.loads(app.config.read_text(encoding='utf-8'));assert 'points' not in app.config.read_text(encoding='utf-8') and not app.config.with_suffix('.tasks.json').exists()
        assert not (peer.root/'task-launches.jsonl').exists()
        passed('Changing server discards prior session data and pause state; samples create no hosted jobs, task history or persisted point data')

    saved['config']['settings']['resource_interval_seconds']=30
    with Desktop(out/'restart',saved) as app:
        app.connect(1);resources(app);eventually(lambda:len(data(app.page)['points'])==1)
        assert data(app.page)['points'][0]['cpu'] is None
        passed('Restarting the client starts new session history and requires fresh samples instead of loading historical monitoring data')

(out/'summary.json').write_text(json.dumps(checks,ensure_ascii=False,indent=2),encoding='utf-8');print(f'{len(checks)} groups passed; artifacts: {out}',flush=True)
