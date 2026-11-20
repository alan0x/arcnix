import { format } from 'prettier/standalone';
import babel from 'prettier/plugins/babel';
import estree from 'prettier/plugins/estree';
import html from 'prettier/plugins/html';
import postcss from 'prettier/plugins/postcss';
import typescript from 'prettier/plugins/typescript';

self.onmessage = async ({ data: { text, parser, ...options } }) => {
  try {
    if (parser === 'json-stringify') JSON.parse(text);
    const formatted = await format(text, {
      ...options,
      parser,
      plugins: [babel, estree, html, postcss, typescript],
      embeddedLanguageFormatting: 'off',
    });
    self.postMessage({ formatted });
  } catch (error) {
    self.postMessage({ error: error.message });
  }
};
