// A zero-based logical line at each nominal SFTP page boundary. Allocate
// checkpoints as the prefix is scanned, independently of the file's size.
const pageBytes = window.arcnixWorkspace.pageBytes;
const segmentSize = 256;
let cursor = 0;
export const nextReadCursor = () => ++cursor;

export class PageLineIndex {
  segments = [];
  next = 0;
  lines = 0;

  append(offset, page) {
    if (offset !== this.next) return;
    const number = offset / pageBytes;
    const slot = number % segmentSize;
    if (slot === 0) this.segments.push(new Float64Array(segmentSize));
    this.lines += page.breaks;
    this.segments[Math.floor(number / segmentSize)][slot] = this.lines;
    this.next = page.next;
  }

  // The former EOF page can change ownership of a CR when LF arrives at
  // the next page boundary. Keep only checkpoints before that page.
  beforeAppend(seam, fork) {
    const index = fork ? new PageLineIndex() : this;
    const next = Math.min(this.next, seam);
    const pages = next / pageBytes;
    const lines = this.lineAt(next);
    const slabs = Math.ceil(pages / segmentSize);
    if (fork) {
      index.segments = this.segments.slice(0, slabs);
      if (pages % segmentSize)
        index.segments[slabs - 1] = index.segments[slabs - 1].slice();
    } else index.segments.length = slabs;
    index.next = next;
    index.lines = lines;
    return index;
  }

  lineAt(offset) {
    if (offset === 0) return 0;
    if (offset > this.next) return undefined;
    const previous = offset / pageBytes - 1;
    return this.segments[Math.floor(previous / segmentSize)][
      previous % segmentSize
    ];
  }
}
