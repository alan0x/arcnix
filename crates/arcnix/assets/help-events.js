// Help is presentation only: no credentials, task state or persistence.
(() => {
  let active = null;
  let pinned = false;
  let closing;
  let sequence = 0;
  const triggerAt = target => target instanceof Element ? target.closest('.help-trigger') : null;
  const panelOf = trigger => trigger.closest('.help-tip')?.querySelector('.help-copy');
  function close() {
    clearTimeout(closing);
    if (active) {
      active.setAttribute('aria-expanded', 'false');
      const panel = panelOf(active);
      panel?.classList.remove('help-visible');
      panel?.setAttribute('aria-hidden', 'true');
    }
    active = null;
    pinned = false;
  }
  function position() {
    if (!active?.isConnected || !active.getClientRects().length || active.closest('[inert]')) {
      close();
      return;
    }
    const panel = panelOf(active);
    const frame = desktopFrame.boundsFor(active);
    const rect = active.getBoundingClientRect();
    const pad = 12;
    const gap = 8;
    const width = panel.offsetWidth;
    const height = panel.offsetHeight;
    const left = Math.max(frame.left + pad, Math.min(rect.left, frame.right - width - pad));
    const below = rect.bottom + gap;
    const above = rect.top - gap - height;
    const preferred = below + height <= frame.bottom - pad ? below : above;
    const top = Math.max(frame.top + pad, Math.min(preferred, frame.bottom - height - pad));
    panel.style.left = `${Math.round(left)}px`;
    panel.style.top = `${Math.round(top)}px`;
  }
  function open(trigger, pin = false) {
    clearTimeout(closing);
    if (active !== trigger) close();
    const panel = panelOf(trigger);
    if (!panel) return;
    if (!panel.id) panel.id = `arcnix-help-${++sequence}`;
    trigger.setAttribute('aria-describedby', panel.id);
    trigger.setAttribute('aria-controls', panel.id);
    trigger.setAttribute('aria-expanded', 'true');
    panel.setAttribute('role', 'tooltip');
    panel.setAttribute('aria-hidden', 'false');
    panel.classList.add('help-visible');
    active = trigger;
    pinned = pin || pinned;
    position();
  }
  document.addEventListener('pointerover', event => {
    const trigger = triggerAt(event.target);
    if (trigger && !trigger.contains(event.relatedTarget)) open(trigger);
    else if (active && panelOf(active)?.contains(event.target)) clearTimeout(closing);
  });
  document.addEventListener('pointerout', event => {
    if (!active || pinned) return;
    const wrapper = active.closest('.help-tip');
    if (wrapper.contains(event.target) && !wrapper.contains(event.relatedTarget)) {
      closing = setTimeout(() => {
        if (active && !pinned && document.activeElement !== active) close();
      }, 140);
    }
  });
  document.addEventListener('focusin', event => {
    const trigger = triggerAt(event.target);
    if (trigger) open(trigger);
    else if (active && !active.closest('.help-tip').contains(event.target)) close();
  });
  document.addEventListener('focusout', event => {
    if (active === event.target && !active.closest('.help-tip').contains(event.relatedTarget)) close();
  });
  document.addEventListener('click', event => {
    const trigger = triggerAt(event.target);
    if (!trigger) return;
    event.preventDefault();
    event.stopPropagation();
    if (active === trigger && pinned) close();
    else open(trigger, true);
  }, true);
  document.addEventListener('pointerdown', event => {
    if (active && !active.closest('.help-tip').contains(event.target)) close();
  }, true);
  document.addEventListener('keydown', event => {
    if (event.key === 'Escape' && active) {
      close();
      event.preventDefault();
      event.stopImmediatePropagation();
    }
  }, true);
  document.addEventListener('scroll', event => {
    if (active && !panelOf(active)?.contains(event.target)) close();
  }, true);
  window.addEventListener('resize', close);
  function observe() {
    new MutationObserver(() => { if (active) position(); }).observe(document.body, {
      childList: true, subtree: true, characterData: true,
      attributes: true, attributeFilter: ['class', 'inert', 'hidden'],
    });
  }
  if (document.body) observe();
  else document.addEventListener('DOMContentLoaded', observe, {once: true});
})();
