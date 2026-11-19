const PREVIEW_CHARS = 240;
const encoder = new TextEncoder();
const decoder = new TextDecoder('utf-8', { ignoreBOM: true });

function header(block) {
  return {
    offset: block.offset,
    line: block.line,
    column: block.column,
    digest: block.digest,
  };
}

function nextPosition(block) {
  let line = block.line;
  let lastBreak = -1;
  for (
    let i = block.text.indexOf('\n');
    i !== -1;
    i = block.text.indexOf('\n', i + 1)
  ) {
    line++;
    lastBreak = i;
  }
  return {
    offset: block.next,
    line,
    column:
      lastBreak < 0
        ? block.column + block.text.length
        : block.text.length - lastBreak - 1,
  };
}

function scanWindow(previous, current, next, pattern, resume, quota, matches) {
  const first = previous ?? current;
  const prefix = previous?.text ?? '';
  const text = prefix + current.text + (next?.text ?? '');
  const end = prefix.length + current.text.length;
  const starts = [0];
  for (let i = text.indexOf('\n'); i !== -1; i = text.indexOf('\n', i + 1))
    starts.push(i + 1);
  function position(index) {
    let lo = 0,
      hi = starts.length;
    while (lo + 1 < hi) {
      const mid = Math.floor((lo + hi) / 2);
      if (starts[mid] <= index) lo = mid;
      else hi = mid;
    }
    return {
      line: first.line + lo,
      column: index - starts[lo] + (lo === 0 ? first.column : 0),
      row: lo,
    };
  }
  pattern.lastIndex = prefix.length + resume;
  let consumed = pattern.lastIndex;
  for (const hit of text.matchAll(pattern)) {
    if (next && hit.index >= end) break;
    const after = hit.index + hit[0].length;
    // A truncated window is not the end of the file for an end anchor.
    if (next && after === text.length && next.next < next.size) continue;
    if (matches.length >= quota) return { limitHit: true, resume: 0 };
    const start = position(hit.index),
      finish = position(after);
    const from = Math.max(starts[start.row], hit.index - 48);
    const to = Math.min(
      starts[start.row + 1] === undefined
        ? text.length
        : starts[start.row + 1] - 1,
      from + PREVIEW_CHARS,
    );
    matches.push({
      // A substring can otherwise keep its entire three-block window alive.
      previewText: decoder.decode(encoder.encode(text.slice(from, to))),
      rangeLocations: [
        {
          source: {
            startLineNumber: start.line,
            startColumn: start.column,
            endLineNumber: finish.line,
            endColumn: finish.column,
          },
          preview: {
            startLineNumber: 0,
            startColumn: hit.index - from,
            endLineNumber: 0,
            endColumn: Math.min(after, to) - from,
          },
        },
      ],
    });
    consumed = after;
  }
  return { resume: Math.max(0, consumed - end), limitHit: false };
}

// Keep one range on each side for split matches, lookaround and cross-line
// expressions. Only byte anchors and bounded result snippets leave this worker.
export async function scanLargeFile(
  path,
  snapshot,
  pattern,
  quota,
  request,
  publish,
) {
  pattern = new RegExp(pattern);
  let count = 0;
  async function read(position) {
    const part = await request('range', path, {
      offset: position.offset,
      snapshot,
    });
    return { ...position, ...part, size: snapshot.size };
  }
  let previous,
    current = await read({ offset: 0, line: 0, column: 0 }),
    resume = 0,
    limitHit = false;
  while (current) {
    const next =
      current.next < snapshot.size
        ? await read(nextPosition(current))
        : undefined;
    const matches = [];
    ({ resume, limitHit } = scanWindow(
      previous,
      current,
      next,
      pattern,
      resume,
      quota - count,
      matches,
    ));
    if (matches.length) {
      count += matches.length;
      publish(matches, {
        line: current.line,
        column: current.column,
        blocks: [previous, current, next].filter(Boolean).map(header),
      });
    }
    if (limitHit) break;
    previous = current;
    current = next;
  }
  return { limitHit, count };
}
