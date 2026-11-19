import { build } from 'esbuild';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { gzipSync } from 'node:zlib';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const vendor = path.resolve(
  here,
  '../../crates/arcnix/assets/vendor/workbench',
);
const assets = new Map();
// VS Code extensions resolve their themes, fonts and translations relative to the module.
const localUrls = {
  name: 'local-resources',
  setup(builder) {
    builder.onLoad({ filter: /\.js$/ }, async ({ path: filename }) => {
      const source = await readFile(filename, 'utf8');
      if (!source.includes('import.meta.url')) return;
      const contents = source.replace(
        /new URL\((['"])(\.\.?\/[^'"]+)\1,\s*import\.meta\.url\)/g,
        (original, quote, relative) => {
          const asset = path.resolve(path.dirname(filename), relative);
          if (!existsSync(asset)) return original;
          const name =
            createHash('sha256')
              .update(path.relative(here, asset).replaceAll('\\', '/'))
              .digest('hex')
              .slice(0, 16) + path.extname(asset);
          assets.set(name, asset);
          return `new URL('/${name}',location.href)`;
        },
      );
      return { contents, loader: 'js', resolveDir: path.dirname(filename) };
    });
  },
};
const result = await build({
  absWorkingDir: here,
  entryPoints: [
    'workbench.js',
    'zh.js',
    'worker.js',
    'grammar-worker.js',
    'search-worker.js',
  ],
  outdir: 'out',
  // Font discovery in the optional ligatures addon requires Node.
  external: ['node:fs/promises', '@xterm/addon-ligatures'],
  bundle: true,
  write: false,
  format: 'esm',
  target: 'chrome125',
  minify: true,
  loader: {
    '.ttf': 'file',
    '.svg': 'dataurl',
    '.png': 'dataurl',
    '.wasm': 'file',
  },
  legalComments: 'external',
  plugins: [localUrls],
});
const files = new Map();
for (const output of result.outputFiles)
  files.set(path.basename(output.path), Buffer.from(output.contents));
for (const [name, source] of assets) files.set(name, await readFile(source));
files.set('index.html', await readFile(path.join(here, 'workbench.html')));
files.set('bootstrap.js', await readFile(path.join(here, 'bootstrap.js')));
const packed = Object.fromEntries(
  [...files]
    .sort(([a], [b]) => a.localeCompare(b, 'en'))
    .map(([k, v]) => [k, v.toString('base64')]),
);
await mkdir(vendor, { recursive: true });
await writeFile(
  path.join(vendor, 'workbench.gz'),
  gzipSync(JSON.stringify(packed), { level: 9 }),
);
