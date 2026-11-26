"""Real WebView motion with native app lifecycle, rapid input and reduced motion."""
import json,time,re
from desktop_fixture import Desktop,Peer,ROOT,expect,eventually
from docker_desktop_smoke import seed

def main():
    out=ROOT/'artifacts/desktop-motion'/f'run-{time.time_ns()}';out.mkdir(parents=True)
    local=out/'local';local.mkdir();checks=[]
    with Peer(out/'peer') as peer:
        (peer.root/'docker-fixture.json').write_text(json.dumps(seed()))
        with Desktop(out/'desktop',peer.config([peer.profile('Desktop motion')],local)) as app:
            p=app.page;errors=[];p.on('pageerror',lambda e:errors.append(str(e)))
            app.connect();app.workspace('Desktop')
            expect(p.locator('.desktop-shortcuts').get_by_role('button', name='Files', exact=True)).to_be_visible()
            app.screenshot('desktop-light.png')
            p.set_viewport_size(dict(width=900,height=650))
            dock=p.locator('.desktop-dock').bounding_box()
            for name in ['Monitor','App Center','Docker']:
                tile=p.locator('.desktop-dock').get_by_role('button',name=name,exact=True).locator('.desktop-app-icon')
                if name == 'Docker' and not tile.count():
                    continue
                box=tile.bounding_box()
                assert dock['x']<=box['x'] and box['x']+box['width']<=dock['x']+dock['width']
            masks=p.locator('.desktop-dock-app .desktop-app-icon').evaluate_all("icons=>icons.map(e=>getComputedStyle(e,'::before').maskImage)")
            assert len(set(masks))==len(masks) and 'none' not in masks
            app.screenshot('desktop-compact.png')
            p.set_viewport_size(dict(width=1440,height=980))
            p.evaluate('''() => {
                window.motionRecords=[];
                window.nativeTransition=document.startViewTransition.bind(document);
                document.startViewTransition=callback=>{
                    const record={kind:document.documentElement.className,ready:false,finished:false,animations:[]};
                    motionRecords.push(record);
                    const transition=nativeTransition(callback);
                    transition.ready.then(()=>{
                        record.ready=true;
                        record.animations=document.getAnimations().map(animation=>({
                            name:animation.animationName||'',pseudo:animation.effect?.pseudoElement||'',
                            duration:animation.effect?.getComputedTiming().duration,
                            frames:animation.effect?.getKeyframes().map(frame=>({opacity:frame.opacity,transform:frame.transform}))
                        })).filter(animation=>animation.name.startsWith('desktop-window'));
                    }).catch(error=>{record.error=String(error);});
                    transition.finished.catch(()=>{}).then(()=>record.finished=true);
                    return transition;
                };
            }''')
            def launch(name):p.locator('.desktop-dock').get_by_role('button',name=name,exact=True).click()
            def settled():eventually(lambda:not p.locator('html').evaluate('el=>el.classList.contains("desktop-motion")'))
            def motion(kind,offset):
                eventually(lambda:p.evaluate('(n)=>motionRecords.length>n&&motionRecords[n].finished',offset))
                record=p.evaluate('(n)=>motionRecords[n]',offset)
                assert 'desktop-motion-'+kind in record['kind'],record
                assert record['ready'] and record['animations'],record
                assert all(0<float(a['duration'])<=250 for a in record['animations']),record
                settled();return record
            p.locator('.desktop-shortcuts').get_by_role('button',name='Terminal',exact=True).dblclick()
            opened=motion('open',0)
            entrance=next(a for a in opened['animations'] if a['name']=='desktop-window-enter')
            assert 'scale(0.88)' in entrance['frames'][0]['transform'],entrance
            reader=p.locator('.terminal-surface .xterm-accessibility-tree');keyboard=p.locator('.terminal-surface .xterm-helper-textarea')
            expect(reader).to_contain_text('Arcnix terminal fixture')
            keyboard.focus();p.keyboard.type('echo MOTION_SESSION_RETAINED');p.keyboard.press('Enter');expect(reader).to_contain_text('MOTION_SESSION_RETAINED')
            surface=p.locator('.terminal-owned-surface').element_handle()
            launch('App Center');motion('switch',1)
            cards = p.locator('.app-center-app-card')
            expect(cards).to_have_count(9)
            expect(cards.filter(has_text='pgAdmin')).to_be_visible()
            expect(p.locator('.desktop-window.active .desktop-window-mark .glyph-app-center')).to_be_visible()
            expect(p.locator('.desktop-shortcuts')).to_be_visible()
            titlebar=p.locator('.desktop-window.active .desktop-window-titlebar').bounding_box()
            actions=p.locator('.desktop-window.active .desktop-window-actions').bounding_box()
            assert 0<=titlebar['x']+titlebar['width']-actions['x']-actions['width']<=16
            app.screenshot('app-center-window.png')
            launch('Terminal')
            expect(p.locator('.desktop-window.app-terminal.active')).to_be_visible()
            assert p.evaluate('motionRecords.length') == 2
            expect(reader).to_contain_text('MOTION_SESSION_RETAINED');assert surface.evaluate('el=>el.isConnected')
            p.locator('.desktop-window.active .window-minimize').click();motion('minimize',2)
            expect(p.locator('.desktop-window.app-terminal')).to_be_hidden()
            launch('Terminal');motion('switch',3);expect(reader).to_contain_text('MOTION_SESSION_RETAINED');assert surface.evaluate('el=>el.isConnected')
            p.locator('.terminal-tab.active').dblclick()
            expect(p.locator('.desktop-window.active')).not_to_have_class(re.compile(r'\bmaximized\b'))
            assert p.evaluate('motionRecords.length')==4
            def double_click_titlebar():
                titlebar=p.locator('.desktop-window.active .desktop-window-titlebar')
                box=titlebar.bounding_box()
                titlebar.dblclick(position=dict(x=box['width']/2,y=box['height']/2))

            p.evaluate('''() => {
                window.resizeRecords = [];
                window.resizeView = () => {
                    const window = document.querySelector('.desktop-window.active');
                    const dock = document.querySelector('.desktop-dock-zone');
                    const wallpaper = document.querySelector('.desktop-wallpaper-art');
                    const box = el => {
                        const {left, top, right, bottom} = el.getBoundingClientRect();
                        return [left, top, right, bottom];
                    };
                    const style = el => {
                        const {opacity, visibility, backgroundImage} = getComputedStyle(el);
                        return [opacity, visibility, backgroundImage];
                    };
                    return {rect: box(window), opacity: getComputedStyle(window).opacity, dock: box(dock),
                            scene: [style(dock), style(wallpaper)]};
                };
                const animate = Element.prototype.animate;
                Element.prototype.animate = function(frames, options) {
                    const animation = animate.call(this, frames, options);
                    if (this.matches('.desktop-window') && frames[0]?.width) {
                        animation.pause();
                        animation.currentTime = 0;
                        resizeRecords.push(animation);
                    }
                    return animation;
                };
            }''')
            def resized(action, finish=True):
                index = p.evaluate('resizeRecords.length')
                before = p.evaluate('resizeView()')
                action()
                eventually(lambda: p.evaluate('n => resizeRecords.length === n + 1', index))
                first = p.evaluate('resizeView()')
                assert all(abs(a - b) <= .05 for a, b in zip(first['rect'], before['rect'])), (first, before)
                assert {k: v for k, v in first.items() if k != 'rect'} == {k: v for k, v in before.items() if k != 'rect'}
                frames = p.evaluate('n => resizeRecords[n].effect.getKeyframes()', index)
                assert set(frames[0]) - {'offset', 'computedOffset', 'easing', 'composite'} == {'left', 'top', 'width', 'height'}, frames
                p.evaluate('n => {resizeRecords[n].currentTime = 70}', index)
                middle = p.evaluate('resizeView()')
                assert middle['rect'] != before['rect']
                assert {k: v for k, v in middle.items() if k != 'rect'} == {k: v for k, v in before.items() if k != 'rect'}
                assert middle['rect'][3] <= middle['dock'][1] + 1
                assert p.evaluate('motionRecords.length') == 4
                assert not p.evaluate('document.documentElement.classList.contains("desktop-motion")')
                if finish:
                    p.evaluate('n => resizeRecords[n].finish()', index)
                    end = p.evaluate('resizeView()')
                    assert end['rect'][3] <= end['dock'][1] + 1
                    for start, mid, stop in zip(before['rect'], middle['rect'], end['rect']):
                        assert min(start, stop) - 1 <= mid <= max(start, stop) + 1

            normal = p.evaluate('resizeView()')
            maximize = p.locator('.desktop-window.active .window-maximize')
            resized(maximize.click)
            expect(p.locator('.desktop-window.active')).to_have_class(re.compile(r'\bmaximized\b'))
            expect(p.locator('.desktop-window.active .window-maximize .glyph-window-restore')).to_be_visible()
            resized(double_click_titlebar)
            expect(p.locator('.desktop-window.active')).not_to_have_class(re.compile(r'\bmaximized\b'))
            assert p.evaluate('resizeView()') == normal
            resized(double_click_titlebar, finish=False)
            resized(maximize.click)
            assert p.evaluate('resizeView()') == normal
            expect(reader).to_contain_text('MOTION_SESSION_RETAINED');assert surface.evaluate('el=>el.isConnected')
            p.locator('.desktop-window.active .window-close').click();motion('close',4)
            expect(p.locator('.desktop-window.app-terminal')).to_be_hidden()
            expect(p.locator('.desktop-window.app-center.active')).to_be_visible()
            checks.append('Open, switch, minimize and close use finite compositor animations; switching/minimizing preserve the terminal DOM and live shell')
            checks.append('Maximize and restore interpolate the live window from its rendered bounds without fading, affecting the Dock, or taking a desktop snapshot')

            launch('Terminal');settled();expect(reader).to_contain_text('Arcnix terminal fixture');expect(reader).not_to_contain_text('MOTION_SESSION_RETAINED')
            p.evaluate('''() => {
                const dock=document.querySelector('.desktop-dock');
                dock.querySelector('.app-center').click();
                dock.querySelector('.app-terminal').click();
                dock.querySelector('.app-settings').click();
            }''')
            expect(p.locator('.desktop-window.app-settings.active')).to_be_visible();settled()
            p.locator('.desktop-window.app-settings .window-close').click()
            expect(p.locator('.desktop-window.app-settings')).to_have_count(0);settled()
            expect(p.locator('.desktop-dock .app-settings')).not_to_have_class(re.compile(r'\brunning\b'))
            launch('Terminal');settled();expect(reader).to_contain_text('Arcnix terminal fixture')
            p.evaluate('''() => {
                document.querySelector('.desktop-dock .app-center').click();
                document.querySelector('.workbench-tab-select[aria-label="Home"]').click();
            }''')
            expect(p.locator('.server-overview')).to_be_visible();settled();expect(p.locator('.server-desktop')).to_be_hidden()
            app.workspace('Desktop');settled()
            checks.append('Rapid launcher clicks preserve native order; an immediate Home click wins over pending motion without reopening Desktop')

            p.emulate_media(reduced_motion='reduce');before=p.evaluate('motionRecords.length')
            before_resizes = p.evaluate('resizeRecords.length')
            launch('Monitor');expect(p.locator('.desktop-window.active .desktop-window-title')).to_have_text('Monitor')
            p.locator('.desktop-window.active .desktop-window-title').dblclick();expect(p.locator('.desktop-window.active')).to_have_class(re.compile(r'\bmaximized\b'))
            p.locator('.desktop-window.active .desktop-window-title').dblclick();expect(p.locator('.desktop-window.active')).not_to_have_class(re.compile(r'\bmaximized\b'))
            assert p.evaluate('motionRecords.length')==before
            assert p.evaluate('resizeRecords.length') == before_resizes
            assert p.evaluate('document.getAnimations().filter(a=>a.animationName?.startsWith("desktop-window")).length')==0
            assert not p.locator('html').evaluate('el=>el.classList.contains("desktop-motion")')
            p.emulate_media(reduced_motion='no-preference')
            p.evaluate('''() => {
                document.startViewTransition=undefined;
                window.fallbackMotion=[];
                const animate=Element.prototype.animate;
                Element.prototype.animate=function(frames,options){
                    if(this.classList.contains('desktop-window-content'))fallbackMotion.push(options.duration);
                    return animate.call(this,frames,options);
                };
            }''')
            launch('Settings')
            expect(p.locator('.desktop-window.active .desktop-window-title')).to_have_text('Settings')
            expect(p.locator('.desktop-settings-tabs').get_by_role('button', name='General', exact=True)).to_be_visible()
            assert p.evaluate('motionRecords.length')==before
            assert p.evaluate('fallbackMotion')==[200]
            p.locator('.desktop-window.active .window-close').click()
            expect(p.locator('.desktop-window.app-settings')).to_have_count(0)
            p.evaluate('() => {document.startViewTransition=()=>{throw new Error("unavailable test capture")};}')
            launch('Settings');expect(p.locator('.desktop-window.active .desktop-window-title')).to_have_text('Settings')
            assert not p.locator('html').evaluate('el=>el.classList.contains("desktop-motion")')
            assert not errors,errors
            checks.append('Reduced motion skips effects; missing/failed browser transition support preserves native app actions without page errors')
            (out/'animations.json').write_text(json.dumps(p.evaluate('motionRecords'),indent=2))
            app.screenshot('desktop-motion.png')
    (out/'results.json').write_text(json.dumps(checks,indent=2));print(json.dumps(dict(output=str(out),checks=checks),indent=2))

if __name__=='__main__':main()
