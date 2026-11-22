// Use the same maximize operation as the rest of the server desktop.
document.addEventListener('loadstart', event => {
    const video = event.target;
    if (video.matches?.('.remote-player-screen video')) {
        video.controlsList.add('nodownload', 'nofullscreen');
        video.disablePictureInPicture = true;
    }
}, true);

// Apply each preference on load and on an explicit preference change. Transport
// controls can still change the volume for just the current video.
(() => {
    const applied = new WeakMap();
    function apply(video) {
        const raw = video.closest('.remote-player')?.querySelector('.player-preferences')?.textContent;
        if (!raw || raw === applied.get(video)?.raw) return;
        const [volume, speed] = JSON.parse(raw);
        const previous = applied.get(video);
        if (volume !== previous?.volume) video.volume = volume / 100;
        if (speed !== previous?.speed) {
            video.defaultPlaybackRate = speed / 100;
            video.playbackRate = speed / 100;
        }
        applied.set(video, {raw, volume, speed});
    }
    document.addEventListener('loadstart', event => {
        if (event.target.matches?.('.remote-player-screen video')) {
            applied.delete(event.target);
            apply(event.target);
        }
    }, true);
    new MutationObserver(records => {
        if (records.some(({target}) => (target.parentElement?.closest('.player-preferences') || target.matches?.('.player-preferences')))) {
            for (const video of document.querySelectorAll('.remote-player-screen video')) apply(video);
        }
    }).observe(document, {subtree:true, childList:true, characterData:true});
})();

// Keep slider thumbs local during a drag; send one SFTP seek on release.
(() => {
    let mounted, last;
    function format(seconds) {
        const n = Math.floor(seconds);
        return Math.floor(n / 60) + ':' + String(n % 60).padStart(2, '0');
    }
    function sync() {
        const controls = document.querySelector('.mpv-controls');
        const bar = controls?.querySelector('.mpv-control-bar');
        const raw = controls?.querySelector('.mpv-state')?.textContent;
        if (!raw || !bar || (bar === mounted && raw === last)) return;
        const state = JSON.parse(raw);
        function send(command) {
            const input = controls.querySelector('.mpv-command');
            input.value = JSON.stringify([JSON.parse(controls.querySelector('.mpv-state').textContent).serial, command]);
            input.dispatchEvent(new Event('input', {bubbles:true}));
        }
        if (bar !== mounted) {
            const play = document.createElement('button');
            play.className = 'mpv-play';
            play.onclick = () => send('toggle');
            const time = document.createElement('output');
            function slider(command) {
                const range = document.createElement('input');
                range.type = 'range';
                range.className = 'mpv-' + command;
                range.min = '0';
                range.step = command === 'seek' ? '.1' : '1';
                range.onpointerdown = () => { range.dataset.dragging = 'true'; };
                range.onpointerup = range.onpointercancel = range.onblur = () => { delete range.dataset.dragging; };
                range.onchange = () => send({[command]:Number(range.value)});
                return range;
            }
            bar.replaceChildren(play, time, slider('seek'), slider('volume'));
            mounted = bar;
        }
        last = raw;
        const [play, time, seek, volume] = bar.children;
        const m = state.media;
        play.textContent = m.paused ? '▶' : 'Ⅱ';
        play.setAttribute('aria-label', m.paused ? state.play : state.pause);
        time.textContent = m.duration > 0 ? format(m.position) + ' / ' + format(m.duration) : format(m.position);
        seek.setAttribute('aria-label',state.seek);
        volume.setAttribute('aria-label',state.volume);
        seek.max = m.duration;
        if (!seek.dataset.dragging) seek.value = m.position;
        if (!volume.dataset.dragging) volume.value = m.volume;
        play.disabled = volume.disabled = !m.loaded;
        seek.disabled = !m.duration;
    }
    new MutationObserver(sync).observe(document, {subtree:true, childList:true, characterData:true});
})();

(() => {
    const playbackTimers = new WeakMap();
    const pictureClicks = new WeakMap();
    function feedback(video, symbol, settled = '') {
        const indicator = video.closest('.remote-player-screen')?.querySelector('.player-feedback');
        if (!indicator) return;
        clearTimeout(playbackTimers.get(video));
        indicator.dataset.symbol = symbol;
        if (symbol === settled) return;
        playbackTimers.set(video, setTimeout(() => {
            if (video.isConnected) indicator.dataset.symbol = settled;
        }, 420));
    }

    document.addEventListener('click', event => {
        const video = event.target;
        if (!video.matches?.('.remote-player-screen video') || event.button !== 0 || !event.detail || !video.readyState) return;
        // The WebView retargets some clicks in the built-in controls to the video.
        if (event.clientY >= video.getBoundingClientRect().bottom - 64) return;
        // Leave a short interval for a double-click to maximize the window.
        clearTimeout(pictureClicks.get(video));
        if (event.detail !== 1) return;
        pictureClicks.set(video, setTimeout(() => {
            if (!video.isConnected) return;
            if (video.paused) video.play().catch(() => feedback(video, 'play', 'play'));
            else video.pause();
        }, 220));
    }, true);

    document.addEventListener('play', event => {
        if (event.target.matches?.('.remote-player-screen video')) feedback(event.target, 'play');
    }, true);
    document.addEventListener('pause', event => {
        if (event.target.matches?.('.remote-player-screen video') && !event.target.ended) feedback(event.target, 'pause', 'play');
    }, true);
    document.addEventListener('ended', event => {
        if (event.target.matches?.('.remote-player-screen video')) feedback(event.target, 'play', 'play');
    }, true);
    document.addEventListener('loadstart', event => {
        if (event.target.matches?.('.remote-player-screen video')) feedback(event.target, '');
    }, true);
})();

function maximizeVideoWindow(event) {
    if (!event.target.matches?.('.remote-player-screen video')) return;
    event.preventDefault();
    event.stopPropagation();
    event.target.closest('.desktop-window')?.querySelector('.window-maximize')?.click();
}
document.addEventListener('dblclick', maximizeVideoWindow, true);
document.addEventListener('keydown', event => {
    if (event.key.toLowerCase() === 'f' && !event.repeat && !event.ctrlKey && !event.altKey && !event.metaKey) {
        maximizeVideoWindow(event);
    }
}, true);
document.addEventListener('contextmenu', event => {
    if (event.target.matches?.('.remote-player-screen video')) event.preventDefault();
}, true);
document.addEventListener('loadedmetadata', event => {
    const video = event.target;
    if (video.matches?.('.remote-player-screen video')) {
        const bridge = video.closest('.remote-player')?.querySelector('.player-opened-input');
        if (bridge) {
            bridge.value = new URL(video.currentSrc).pathname;
            bridge.dispatchEvent(new Event('input', { bubbles: true }));
        }
        video.play().catch(() => {});
    }
}, true);
document.addEventListener('error', event => {
    if (event.target.matches?.('.remote-player-screen video[src]')) {
        event.target.classList.add('playback-failed');
    }
}, true);
