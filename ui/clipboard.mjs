// Copy annotations to the system clipboard so they can be pasted into any
// application (documents, slides, PDF editors, chat apps…).
//
// Formats (Settings → Clipboard):
//   rich    — PNG + HTML (images with captions) + plain text. Rich editors get
//             images and notes; image-only targets get the screenshot.
//   image   — the annotated screenshot only.
//   caption — one PNG with the note printed under the screenshot.
//   text    — the notes only.
// Several annotations are stacked into one image (with captions) where a single
// image is required.

export function itemText(item, index) {
  const marks = item.marks?.length > 1 ? item.marks.map((m, n) => `${n + 1}. ${m.comment.trim()}`).join('\n') : item.comment.trim();
  return `#${index + 1} — ${item.source}\n${marks}`;
}

const escapeHtml = value => String(value).replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

export function htmlFor(entries) {
  const figures = entries.map(({ item, index, image }) => {
    const notes = item.marks?.length > 1
      ? `<ol>${item.marks.map(m => `<li>${escapeHtml(m.comment.trim())}</li>`).join('')}</ol>`
      : `<p>${escapeHtml(item.comment.trim()).replace(/\n/g, '<br>')}</p>`;
    return `<figure><img src="${image}" alt="Annotation ${index + 1}" style="max-width:100%"><figcaption><strong>#${index + 1} — ${escapeHtml(item.source)}</strong>${notes}</figcaption></figure>`;
  });
  return `<meta charset="utf-8"><div>${figures.join('\n')}</div>`;
}

const load = src => new Promise((resolve, reject) => { const image = new Image(); image.onload = () => resolve(image); image.onerror = () => reject(new Error('Cannot read the screenshot')); image.src = src; });
const toBlob = canvas => new Promise((resolve, reject) => canvas.toBlob(blob => (blob ? resolve(blob) : reject(new Error('Cannot encode the image'))), 'image/png'));

function wrap(ctx, text, width) {
  const lines = [];
  for (const paragraph of String(text).split('\n')) {
    let line = '';
    for (const word of paragraph.split(/\s+/)) {
      const next = line ? `${line} ${word}` : word;
      if (ctx.measureText(next).width > width && line) { lines.push(line); line = word; } else line = next;
    }
    lines.push(line);
  }
  return lines;
}

/** Stacks screenshots vertically, optionally with a caption under each. */
export async function composeImage(entries, captions) {
  const images = await Promise.all(entries.map(e => load(e.image)));
  const width = Math.max(...images.map(i => i.naturalWidth));
  const font = Math.max(14, Math.round(width / 70)), pad = Math.round(font * 0.9), line = Math.round(font * 1.35);
  const measure = document.createElement('canvas').getContext('2d');
  measure.font = `${font}px system-ui, sans-serif`;
  const blocks = entries.map((e, i) => {
    const lines = captions ? wrap(measure, itemText(e.item, e.index), width - pad * 2) : [];
    return { image: images[i], lines, height: images[i].naturalHeight + (lines.length ? lines.length * line + pad * 2 : 0) };
  });
  const gap = entries.length > 1 ? pad : 0;
  const canvas = document.createElement('canvas');
  canvas.width = width; canvas.height = blocks.reduce((sum, b) => sum + b.height, 0) + gap * (blocks.length - 1);
  const ctx = canvas.getContext('2d');
  ctx.fillStyle = '#ffffff'; ctx.fillRect(0, 0, canvas.width, canvas.height);
  let y = 0;
  for (const block of blocks) {
    ctx.drawImage(block.image, 0, y);
    y += block.image.naturalHeight;
    if (block.lines.length) {
      ctx.fillStyle = '#f4f6fb'; ctx.fillRect(0, y, width, block.lines.length * line + pad * 2);
      ctx.fillStyle = '#161b26'; ctx.font = `${font}px system-ui, sans-serif`; ctx.textBaseline = 'top';
      block.lines.forEach((text, n) => {
        ctx.font = `${n === 0 ? '600 ' : ''}${font}px system-ui, sans-serif`;
        ctx.fillText(text, pad, y + pad + n * line);
      });
      y += block.lines.length * line + pad * 2;
    }
    y += gap;
  }
  return toBlob(canvas);
}

const dataUrlToBlob = async url => {
  const [, b64] = url.split(',');
  const bytes = Uint8Array.from(atob(b64), c => c.charCodeAt(0));
  return new Blob([bytes], { type: 'image/png' });
};
const blobToDataUrl = blob => new Promise(resolve => { const r = new FileReader(); r.onload = () => resolve(r.result); r.readAsDataURL(blob); });

/**
 * Copies annotations. `entries` is a promise of [{ item, index, image(dataURL) }]
 * so the ClipboardItem can be created synchronously inside the click.
 * `fallback(kind, data, alt)` writes through the native clipboard.
 */
export async function copyAnnotations(entriesPromise, format, fallback) {
  const text = entriesPromise.then(entries => entries.map(e => itemText(e.item, e.index)).join('\n\n'));
  const png = format === 'text' ? null : entriesPromise.then(entries => (entries.length === 1 && format !== 'caption' ? dataUrlToBlob(entries[0].image) : composeImage(entries, format !== 'image')));
  const html = format === 'rich' ? entriesPromise.then(htmlFor) : null;
  const parts = {};
  if (png) parts['image/png'] = png;
  if (format !== 'image') parts['text/plain'] = text.then(t => new Blob([t], { type: 'text/plain' }));
  if (html) parts['text/html'] = html.then(h => new Blob([h], { type: 'text/html' }));
  try {
    if (!window.ClipboardItem || !navigator.clipboard?.write) throw new Error('Web clipboard unavailable');
    await navigator.clipboard.write([new ClipboardItem(parts)]);
    return describe(format, await entriesPromise);
  } catch {
    // Native fallback: one format only.
    if (png) await fallback('image', await blobToDataUrl(await png));
    else await fallback('text', await text);
    return `${describe(format, await entriesPromise)}${png && format !== 'image' ? ' (image only — this desktop’s clipboard could not take the notes too)' : ''}`;
  }
}

function describe(format, entries) {
  const what = entries.length === 1 ? `annotation #${entries[0].index + 1}` : `${entries.length} annotations`;
  return { rich: `Copied ${what}: screenshot and notes. Paste into any app.`, image: `Copied the screenshot of ${what}.`, caption: `Copied ${what} as an image with its note.`, text: `Copied the notes of ${what}.` }[format];
}
