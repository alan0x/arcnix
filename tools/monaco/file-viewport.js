import { nextReadCursor } from './line-index.js';

// The native scrollbar spans the snapshot. Unread pages use estimated heights,
// while the loaded window uses measured rows and renders only the viewport.
const rowHeight = 20;
const rowWidth = 2048;
const pageBytes = window.arcnixWorkspace.pageBytes;

// All panes share one SSH worker. Let viewport reads through before issuing
// another count, including when another split has a large file open.
let foregroundReads = 0;
let counting = false;

function indexAt(starts, value) {
  let low = 0,
    high = starts.length;
  while (low + 1 < high) {
    const mid = Math.floor((low + high) / 2);
    if (starts[mid] <= value) low = mid;
    else high = mid;
  }
  return low;
}

function layout(parts, known, lineIndex) {
  const text = parts.map((part) => part.text).join('');
  const pages = [],
    lines = [0],
    rows = [0];
  let length = 0;
  for (const part of parts) {
    pages.push(length);
    length += part.text.length;
  }
  let row = 0;
  for (let i = 0; i < text.length; i++) {
    if (text[i] === '\n') {
      row = i + 1;
      lines.push(row);
      rows.push(row);
    } else if (
      i - row >= rowWidth &&
      !(text.charCodeAt(i) >= 0xdc00 && text.charCodeAt(i) <= 0xdfff)
    ) {
      row = i;
      rows.push(row);
    }
  }
  const ref = parts.findIndex((part) => known.has(part.offset));
  let line, column;
  if (ref >= 0) {
    const pos = known.get(parts[ref].offset);
    const rel = indexAt(lines, pages[ref]);
    line = pos.line - rel;
    column =
      rel === 0 && pos.column != null ? pos.column - pages[ref] : undefined;
    for (const [i, part] of parts.entries()) {
      const at = pages[i],
        rel = indexAt(lines, at);
      known.set(part.offset, {
        line: line + rel,
        column: rel
          ? at - lines[rel]
          : column == null
            ? undefined
            : column + at,
      });
    }
  }
  return {
    text,
    pages,
    lines,
    rows,
    line: line ?? lineIndex.lineAt(parts[0].offset),
    column,
  };
}

export class FileViewport {
  constructor(
    container,
    request,
    path,
    snapshot,
    zh,
    active,
    known,
    lineIndex,
    stamp,
    adopt,
    remember,
  ) {
    Object.assign(this, {
      container,
      request,
      path,
      snapshot,
      zh,
      active,
      known,
      lineIndex,
      stamp,
      adopt,
      remember,
    });
    this.count = Math.ceil(snapshot.size / pageBytes);
    this.body = document.createElement('div');
    this.body.tabIndex = 0;
    this.body.className = 'paged-lines';
    this.sheet = document.createElement('div');
    this.sheet.className = 'paged-sheet';
    this.body.append(this.sheet);
    this.title = document.createElement('header');
    this.title.textContent = zh ? '正在读取…' : 'Loading…';
    container.replaceChildren(this.title, this.body);
    this.body.onscroll = () => this.update();
    // The enclosing VS Code pane also owns a scrollable element. Route wheel
    // deltas to this virtual document before that element consumes them.
    this.body.addEventListener(
      'wheel',
      (event) => {
        if (event.ctrlKey || event.metaKey) return;
        const unit =
          event.deltaMode === WheelEvent.DOM_DELTA_LINE
            ? rowHeight
            : event.deltaMode === WheelEvent.DOM_DELTA_PAGE
              ? this.body.clientHeight
              : 1;
        if (event.shiftKey)
          this.body.scrollLeft += (event.deltaX || event.deltaY) * unit;
        else {
          this.body.scrollLeft += event.deltaX * unit;
          this.body.scrollTop += event.deltaY * unit;
        }
        event.preventDefault();
        event.stopPropagation();
      },
      { passive: false },
    );
    this.body.onkeydown = (event) => {
      if (event.shiftKey || event.altKey) return;
      const delta = {
        ArrowUp: -rowHeight,
        ArrowDown: rowHeight,
        PageUp: -this.body.clientHeight,
        PageDown: this.body.clientHeight,
      }[event.key];
      if (event.key === 'Home') this.body.scrollTop = 0;
      else if (event.key === 'End')
        this.body.scrollTop = this.body.scrollHeight;
      else if (delta !== undefined) this.body.scrollTop += delta;
      else return;
      event.preventDefault();
      event.stopPropagation();
    };
    this.resize = new ResizeObserver(() => this.update());
    this.resize.observe(this.body);
  }
  dispose() {
    this.closed = true;
    clearTimeout(this.indexTimer);
    clearTimeout(this.pollTimer);
    this.releaseCursor();
    this.resize.disconnect();
    this.body.onscroll = this.body.onkeydown = null;
  }
  get live() {
    return !this.closed && this.active();
  }
  async read(offset, digest, snapshot = this.snapshot) {
    foregroundReads++;
    let part;
    try {
      part = await this.request('readRange', this.path, {
        offset,
        snapshot,
      });
    } finally {
      foregroundReads--;
    }
    if (digest && String(digest) !== String(part.digest))
      throw new Error(
        this.zh
          ? '文件已变化，请重新搜索。'
          : 'File changed. Run the search again.',
      );
    if (snapshot === this.snapshot) this.lineIndex.append(offset, part);
    return { offset, ...part };
  }
  async open(anchor, range, saved) {
    this.range = range;
    const parts = [];
    const blocks =
      anchor?.blocks ??
      (saved?.tail
        ? this.tailOffsets(this.count).map((offset) => ({ offset }))
        : [{ offset: saved?.offset ?? 0 }]);
    for (const block of blocks) {
      if (!this.live) return;
      if (block.line != null) this.known.set(block.offset, block);
      parts.push(await this.read(block.offset, block.digest));
    }
    if (!this.live) return;
    this.install(parts);
    let row = 0;
    if (range && this.view.line != null) {
      const local = range.startLineNumber - this.view.line;
      const at =
        this.view.lines[local] +
        range.startColumn -
        (local === 0 ? (this.view.column ?? 0) : 0);
      row = indexAt(this.view.rows, at);
    } else if (saved) row = indexAt(this.view.rows, saved.char);
    this.body.scrollTop =
      this.top + row * rowHeight - (range ? this.body.clientHeight / 2 : 0);
    if (!anchor && saved?.tail) this.body.scrollTop = this.body.scrollHeight;
    this.paint();
    this.schedulePoll();
    const mark = this.sheet.querySelector('mark');
    if (mark && mark.offsetLeft > this.body.clientWidth)
      this.body.scrollLeft = Math.max(
        0,
        mark.offsetLeft - this.body.clientWidth / 2,
      );
  }
  tailOffsets(count) {
    const last = count - 1;
    return (last > 0 ? [last - 1, last] : [last]).map(
      (page) => page * pageBytes,
    );
  }
  atBottom() {
    return (
      this.body.scrollTop + this.body.clientHeight >= this.body.scrollHeight - 2
    );
  }
  schedulePoll(delay = 1000) {
    if (this.live && !this.failed)
      this.pollTimer = setTimeout(() => void this.pollGrowth(), delay);
  }
  async pollGrowth() {
    if (!this.live || this.failed) return;
    let delay = 1000;
    try {
      if (!this.body.clientHeight || document.hidden || this.loading) return;
      const update = await this.request('pollTail', this.path, {
        snapshot: this.snapshot,
        stamp: this.stamp,
      });
      if (
        !this.live ||
        this.failed ||
        update.snapshot.size === this.snapshot.size
      )
        return;
      if (!(await this.applyGrowth(update))) delay = 100;
    } catch (error) {
      if (!this.live) return;
      if (/file changed|linked paths/i.test(error.message)) {
        this.failed = true;
        this.releaseCursor();
        this.title.textContent = this.zh
          ? '文件已变化，请重新打开。'
          : 'File changed. Reopen the file.';
      } else {
        delay = 3000;
        this.title.textContent = this.zh
          ? '正在重试读取日志…'
          : 'Retrying log update…';
      }
    } finally {
      this.schedulePoll(delay);
    }
  }
  async applyGrowth({ snapshot, stamp }) {
    if (this.loading || this.destination() != null) return false;
    const previous = this.parts;
    const follow = this.atBottom();
    const seam = Math.floor((this.snapshot.size - 1) / pageBytes) * pageBytes;
    const count = Math.ceil(snapshot.size / pageBytes);
    const offsets = follow
      ? this.tailOffsets(count)
      : previous.map((p) => p.offset);
    const parts = [];
    for (const offset of offsets) {
      const cached = offset < seam && previous.find((p) => p.offset === offset);
      parts.push(cached || (await this.read(offset, undefined, snapshot)));
      // A wheel event, scrollbar drag or tab change can occur during the read.
      if (
        !this.live ||
        this.failed ||
        this.loading ||
        this.parts !== previous ||
        follow !== this.atBottom() ||
        this.destination() != null
      )
        return false;
    }
    const oldTop = this.top;
    const relative = this.body.scrollTop - oldTop;
    this.lineIndex = this.lineIndex.beforeAppend(seam, !this.growing);
    this.growing = true;
    this.count = count;
    this.snapshot = snapshot;
    this.stamp = stamp;
    this.adopt(snapshot, stamp, this.lineIndex);
    for (const part of parts) this.lineIndex.append(part.offset, part);
    if (
      parts.length === previous.length &&
      parts.every((part, i) => part === previous[i])
    ) {
      // Only the scrollbar extent changed. Retain the reader's DOM selection.
      this.sizeSheet();
      if (this.top !== oldTop) this.rendered = '';
      this.scheduleIndex(60);
    } else this.install(parts);
    this.body.scrollTop = follow ? this.body.scrollHeight : this.top + relative;
    this.paint();
    return true;
  }
  install(parts) {
    this.parts = parts;
    this.first = parts[0].offset / pageBytes;
    this.end = this.first + parts.length;
    this.view = layout(parts, this.known, this.lineIndex);
    this.scheduleIndex(60);
    this.sizeSheet();
    this.rendered = '';
  }
  sizeSheet() {
    if (!this.stride)
      this.stride = Math.min(
        6_000_000 / this.count,
        Math.max(400, (this.view.rows.length * rowHeight) / this.parts.length),
      );
    this.stride = Math.min(this.stride, 6_000_000 / this.count);
    this.top = this.first * this.stride;
    this.bottom = this.top + this.view.rows.length * rowHeight;
    this.sheet.style.height =
      Math.ceil(this.bottom + (this.count - this.end) * this.stride) + 'px';
  }
  releaseCursor() {
    if (this.indexCursor === undefined) return;
    const cursor = this.indexCursor;
    this.indexCursor = undefined;
    // Closing a pane or dropping a session can race this best-effort release.
    void this.request('closeRange', '', { cursor }).catch(() => {});
  }
  needsIndex() {
    return (
      this.view.line == null ||
      (this.growing && this.lineIndex.next < this.snapshot.size)
    );
  }
  scheduleIndex(delay) {
    if (this.indexTimer || this.indexPending || this.indexError || !this.live)
      return;
    if (!this.needsIndex()) {
      this.releaseCursor();
      return;
    }
    this.indexTimer = setTimeout(() => {
      this.indexTimer = undefined;
      void this.advanceIndex();
    }, delay);
  }
  async advanceIndex() {
    if (!this.live || this.failed || !this.needsIndex()) {
      this.releaseCursor();
      return;
    }
    if (
      counting ||
      foregroundReads ||
      this.loading ||
      this.destination() != null
    ) {
      this.scheduleIndex(30);
      return;
    }
    const offset = this.lineIndex.next;
    const snapshot = this.snapshot;
    try {
      const resident = this.parts.find((part) => part.offset === offset);
      if (resident) this.lineIndex.append(offset, resident);
      else if (offset < (this.growing ? snapshot.size : this.parts[0].offset)) {
        counting = true;
        this.indexPending = true;
        this.indexCursor ??= nextReadCursor();
        const metadata = await this.request('countLines', this.path, {
          offset,
          snapshot,
          cursor: this.indexCursor,
        });
        if (!this.live || this.failed) return;
        // Counts from an earlier poll remain valid strictly before its EOF page.
        if (
          snapshot === this.snapshot ||
          metadata.next <=
            Math.floor((snapshot.size - 1) / pageBytes) * pageBytes
        )
          this.lineIndex.append(offset, metadata);
      }
      // A drag may have replaced the window while the SFTP read was pending.
      // Relabel the current rows in place without touching the scroll position.
      if (this.view.line == null) {
        const line = this.lineIndex.lineAt(this.parts[0].offset);
        if (line != null) {
          this.view.line = line;
          for (const part of this.parts)
            this.lineIndex.append(part.offset, part);
          this.rendered = '';
          this.paint();
        }
      }
    } catch (error) {
      if (this.live && snapshot === this.snapshot && !this.failed) {
        this.indexError = error.message;
        this.paint();
      }
    } finally {
      if (this.indexPending) {
        this.indexPending = false;
        counting = false;
      }
      if (!this.live || this.failed || !this.needsIndex() || this.indexError)
        this.releaseCursor();
      else this.scheduleIndex(0);
    }
  }
  position(row) {
    const char = this.view.rows[row];
    const page = indexAt(this.view.pages, char);
    return {
      offset: this.parts[page].offset,
      char: char - this.view.pages[page],
    };
  }
  paint() {
    const { view, body } = this;
    const relative = (body.scrollTop - this.top) / rowHeight;
    const first = Math.max(0, Math.floor(relative) - 5);
    const end = Math.min(
      view.rows.length,
      Math.ceil(relative + body.clientHeight / rowHeight) + 5,
    );
    if (end <= first) {
      this.title.textContent = this.zh ? '正在读取…' : 'Loading…';
      this.sheet.replaceChildren();
      this.rendered = '';
      return;
    }
    const current = Math.max(
      0,
      Math.min(view.rows.length - 1, Math.floor(relative)),
    );
    const position = this.position(current);
    this.remember({
      ...position,
      tail: this.end === this.count && this.atBottom(),
    });
    const line =
      view.line == null
        ? undefined
        : view.line + indexAt(view.lines, view.rows[current]) + 1;
    const location =
      line == null
        ? (this.indexError ??
          (this.zh ? '正在计算行号…' : 'Indexing line numbers…'))
        : this.zh
          ? '第 ' + line + ' 行'
          : 'Line ' + line;
    this.title.textContent = location + (this.zh ? ' · 只读' : ' · Read-only');
    const key = first + '-' + end;
    if (this.rendered === key) return;
    this.rendered = key;
    const fragment = document.createDocumentFragment();
    for (let row = first; row < end; row++) {
      const at = view.rows[row],
        local = indexAt(view.lines, at);
      const number = view.line == null ? undefined : view.line + local;
      const column = at - view.lines[local] + (!local ? (view.column ?? 0) : 0);
      let value = view.text.slice(at, view.rows[row + 1] ?? view.text.length);
      if (value.endsWith('\n')) value = value.slice(0, -1);
      const el = document.createElement('div');
      el.className = 'paged-row';
      el.style.top = this.top + row * rowHeight + 'px';
      const gutter = document.createElement('span');
      gutter.className = 'paged-number';
      if (number != null) el.dataset.line = String(number + 1);
      gutter.textContent =
        at !== view.lines[local]
          ? '↪'
          : number == null
            ? '·'
            : String(number + 1);
      const code = document.createElement('code');
      const hit = this.range;
      if (hit && number >= hit.startLineNumber && number <= hit.endLineNumber) {
        const start =
          number === hit.startLineNumber
            ? Math.max(0, hit.startColumn - column)
            : 0;
        const finish =
          number === hit.endLineNumber
            ? Math.min(value.length, hit.endColumn - column)
            : value.length;
        if (start <= finish && start <= value.length) {
          const mark = document.createElement('mark');
          mark.textContent = value.slice(start, finish);
          code.append(value.slice(0, start), mark, value.slice(finish));
        } else code.textContent = value;
      } else code.textContent = value;
      el.append(gutter, code);
      fragment.append(el);
    }
    const pinned = this.atBottom();
    this.sheet.replaceChildren(fragment);
    // A newly rendered long row can introduce a horizontal scrollbar and
    // reduce clientHeight after End already positioned the vertical thumb.
    if (pinned) body.scrollTop = body.scrollHeight;
  }
  destination() {
    const y = this.body.scrollTop,
      h = this.body.clientHeight;
    if (this.first && y === 0) return 0;
    if (this.end < this.count && y + h >= this.body.scrollHeight - 1)
      return this.count - 1;
    if (this.first && y < this.top + rowHeight)
      return Math.max(0, Math.min(this.first - 1, Math.floor(y / this.stride)));
    if (this.end < this.count && y + h >= this.bottom - rowHeight)
      return Math.min(
        this.count - 1,
        this.end + Math.max(0, Math.floor((y - this.bottom) / this.stride)),
      );
  }
  async update() {
    if (!this.live || !this.view || this.failed) return;
    this.paint();
    const page = this.destination();
    if (this.loading || page == null) return;
    this.loading = true;
    try {
      const part = await this.read(page * pageBytes);
      if (!this.live || page !== this.destination()) return;
      const y = this.body.scrollTop;
      const adjacent = page === this.first - 1 || page === this.end;
      const at = Math.floor((y - this.top) / rowHeight);
      // Pin a character in the overlapping text when replacing a neighbour.
      const pinnedRow = Math.min(Math.max(0, at), this.view.rows.length - 1);
      const pin = this.position(pinnedRow);
      const delta = y - (this.top + pinnedRow * rowHeight);
      const start =
        page < this.first
          ? page * this.stride
          : this.bottom + (page - this.end) * this.stride;
      const fraction = Math.max(0, Math.min(1, (y - start) / this.stride));
      const atEnd = this.atBottom();
      const parts = !adjacent
        ? [part]
        : page < this.first
          ? [part, ...this.parts.slice(0, 2)]
          : [...this.parts.slice(-2), part];
      this.install(parts);
      const old = parts.findIndex((p) => p.offset === pin.offset);
      const target =
        old >= 0
          ? this.top +
            indexAt(this.view.rows, this.view.pages[old] + pin.char) *
              rowHeight +
            delta
          : this.top + fraction * this.view.rows.length * rowHeight;
      this.body.scrollTop =
        atEnd && this.end === this.count ? this.body.scrollHeight : target;
      this.paint();
    } catch (error) {
      if (this.live) {
        this.failed = true;
        this.title.textContent = error.message;
      }
    } finally {
      this.loading = false;
      if (this.live && !this.failed && this.destination() != null)
        requestAnimationFrame(() => this.update());
    }
  }
}
