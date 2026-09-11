const assert = require('node:assert/strict');

function luminance(hex) {
  if (/^#[0-9a-f]{3}$/i.test(hex)) hex = '#' + [...hex.slice(1)].map(value => value + value).join('');
  assert(/^#[0-9a-f]{6}$/i.test(hex), `Unexpected color: ${hex}`);
  const channels = [1, 3, 5].map(index => parseInt(hex.slice(index, index + 2), 16) / 255)
    .map(channel => channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4);
  return channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722;
}

module.exports = async function contrast(page) {
  const pairs = [
    ['foreground', 'background', 4.5], ['secondary-foreground', 'card', 4.5],
    ['muted-foreground', 'card', 4.5], ['primary-foreground', 'primary', 4.5],
    ['success', 'success-bg', 4.5], ['warning', 'warning-bg', 4.5],
    ['destructive', 'destructive-bg', 4.5], ['info', 'info-bg', 4.5],
    ['input', 'card', 3], ['ring', 'background', 3],
  ];
  const colors = await page.evaluate(pairs => Object.fromEntries(pairs.flatMap(([a, b]) => [a, b]).map(key => [key, getComputedStyle(document.documentElement).getPropertyValue(`--${key}`).trim()])), pairs);
  return Object.fromEntries(pairs.map(([a, b, minimum]) => {
    const [light, dark] = [luminance(colors[a]), luminance(colors[b])].sort((a, b) => b - a);
    const value = (light + 0.05) / (dark + 0.05);
    assert(value >= minimum, `${a}/${b}: ${value} < ${minimum}`);
    return [`${a}/${b}`, Number(value.toFixed(2))];
  }));
};
