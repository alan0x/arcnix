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
            expect(p.locator('.desktop-shortcuts button')).to_have_count(8)
            app.screenshot('desktop-light.png')
            p.set_viewport_size(dict(width=900,height=650))
            dock=p.locator('.desktop-dock').bounding_box()
            for name in ['Monitor','Software store','Docker']:
                tile=p.locator('.desktop-dock').get_by_role('button',name=name,exact=True).locator('.desktop-app-icon')
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
            p.locator('.desktop-shortcuts').get_by_role('button',name='Terminal',exact=True).click()
            opened=motion('open',0)
            entrance=next(a for a in opened['animations'] if a['name']=='desktop-window-enter')
            assert 'scale(0.88)' in entrance['frames'][0]['transform'],entrance
            reader=p.locator('.terminal-surface .xterm-accessibility-tree');keyboard=p.locator('.terminal-surface .xterm-helper-textarea')
            expect(reader).to_contain_text('Arcnix terminal fixture')
            keyboard.focus();p.keyboard.type('echo MOTION_SESSION_RETAINED');p.keyboard.press('Enter');expect(reader).to_contain_text('MOTION_SESSION_RETAINED')
            surface=p.locator('.terminal-owned-surface').element_handle()
            launch('Software store');motion('switch',1);expect(p.locator('.store-app-card')).to_have_count(18)
            expect(p.locator('.desktop-window-mark .glyph-store')).to_be_visible()
            expect(p.locator('.desktop-shortcuts')).to_be_visible()
            titlebar=p.locator('.desktop-window-titlebar').bounding_box()
            actions=p.locator('.desktop-window-actions').bounding_box()
            assert 0<=titlebar['x']+titlebar['width']-actions['x']-actions['width']<=16
            app.screenshot('store-window.png')
            launch('Terminal');motion('switch',2);expect(reader).to_contain_text('MOTION_SESSION_RETAINED');assert surface.evaluate('el=>el.isConnected')
            p.locator('.window-minimize').click();motion('minimize',3);expect(p.locator('.desktop-window')).to_be_hidden()
            launch('Terminal');motion('open',4);expect(reader).to_contain_text('MOTION_SESSION_RETAINED');assert surface.evaluate('el=>el.isConnected')
            p.locator('.terminal-tab.active').dblclick()
            expect(p.locator('.desktop-window')).to_have_class('desktop-window')
            assert p.evaluate('motionRecords.length')==5
            def double_click_titlebar():
                titlebar=p.locator('.desktop-window-titlebar')
                box=titlebar.bounding_box()
                titlebar.dblclick(position=dict(x=box['width']/2,y=box['height']/2))
            double_click_titlebar();motion('resize',5);expect(p.locator('.desktop-window')).to_have_class('desktop-window maximized')
            expect(p.locator('.window-maximize .glyph-window-restore')).to_be_visible()
            double_click_titlebar();motion('resize',6);expect(p.locator('.desktop-window')).to_have_class('desktop-window')
            expect(reader).to_contain_text('MOTION_SESSION_RETAINED');assert surface.evaluate('el=>el.isConnected')
            p.locator('.window-close').click();motion('close',7);expect(p.locator('.desktop-window')).to_be_hidden()
            checks.append('Open, switch, minimize, maximize and close use finite compositor animations; switching/minimizing preserve the terminal DOM and live shell')
            checks.append('Double-clicking blank titlebar space toggles maximize and restore through the same animation, while terminal tabs retain their own action')

            launch('Terminal');settled();expect(reader).to_contain_text('Arcnix terminal fixture');expect(reader).not_to_contain_text('MOTION_SESSION_RETAINED')
            p.evaluate('''() => {
                const dock=document.querySelector('.desktop-dock');
                dock.querySelector('.app-software').click();
                dock.querySelector('.app-terminal').click();
                dock.querySelector('.app-operations').click();
                document.querySelector('.window-close').click();
            }''')
            expect(p.locator('.desktop-window')).to_be_hidden();settled()
            expect(p.locator('.desktop-dock .app-operations')).not_to_have_class(re.compile(r'\brunning\b'))
            launch('Terminal');settled();expect(reader).to_contain_text('Arcnix terminal fixture')
            p.evaluate('''() => {
                document.querySelector('.desktop-dock .app-software').click();
                document.querySelector('.workbench-tab-select[aria-label="Home"]').click();
            }''')
            expect(p.locator('.server-overview')).to_be_visible();settled();expect(p.locator('.server-desktop')).to_be_hidden()
            app.workspace('Desktop');settled()
            checks.append('Rapid launcher/close clicks preserve native order; an immediate Home click wins over pending motion without reopening Desktop')

            p.emulate_media(reduced_motion='reduce');before=p.evaluate('motionRecords.length')
            launch('Monitor');expect(p.locator('.desktop-window-title')).to_have_text('Monitor')
            p.locator('.desktop-window-title').dblclick();expect(p.locator('.desktop-window')).to_have_class('desktop-window maximized')
            p.locator('.desktop-window-title').dblclick();expect(p.locator('.desktop-window')).to_have_class('desktop-window')
            assert p.evaluate('motionRecords.length')==before
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
            launch('Services');expect(p.locator('.desktop-window-title')).to_have_text('Services');expect(p.locator('.service-row')).to_have_count(4)
            assert p.evaluate('motionRecords.length')==before
            assert p.evaluate('fallbackMotion')==[200]
            p.evaluate('() => {document.startViewTransition=()=>{throw new Error("unavailable test capture")};}')
            launch('Monitor');expect(p.locator('.desktop-window-title')).to_have_text('Monitor')
            assert not p.locator('html').evaluate('el=>el.classList.contains("desktop-motion")')
            assert not errors,errors
            checks.append('Reduced motion skips effects; missing/failed browser transition support preserves native app actions without page errors')
            (out/'animations.json').write_text(json.dumps(p.evaluate('motionRecords'),indent=2))
            app.screenshot('desktop-motion.png')
    (out/'results.json').write_text(json.dumps(checks,indent=2));print(json.dumps(dict(output=str(out),checks=checks),indent=2))

if __name__=='__main__':main()
