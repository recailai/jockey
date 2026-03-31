const PALETTES = [
  ["#6366f1", "#818cf8"],
  ["#8b5cf6", "#a78bfa"],
  ["#ec4899", "#f472b6"],
  ["#f43f5e", "#fb7185"],
  ["#f97316", "#fb923c"],
  ["#eab308", "#facc15"],
  ["#22c55e", "#4ade80"],
  ["#14b8a6", "#2dd4bf"],
  ["#06b6d4", "#22d3ee"],
  ["#3b82f6", "#60a5fa"],
];

function hash(str: string): number {
  let h = 0;
  for (let i = 0; i < str.length; i++) {
    h = ((h << 5) - h + str.charCodeAt(i)) | 0;
  }
  return Math.abs(h);
}

export function identicon(seed: string, size = 32): string {
  const h = hash(seed);
  const h2 = hash(seed + seed);
  const palette = PALETTES[h % PALETTES.length];
  const fg = palette[0];
  const hi = palette[1];
  const grid = 7;
  const half = Math.ceil(grid / 2);
  const gap = 0.5;
  const cellSize = (size - gap * (grid + 1)) / grid;

  const bits: boolean[][] = [];
  for (let row = 0; row < grid; row++) {
    bits[row] = [];
    for (let col = 0; col < half; col++) {
      const idx = row * half + col;
      const src = idx < 30 ? h : h2;
      bits[row][col] = ((src >> (idx % 30)) & 1) === 1;
    }
    for (let col = half; col < grid; col++) {
      bits[row][col] = bits[row][grid - 1 - col];
    }
  }

  let rects = "";
  for (let row = 0; row < grid; row++) {
    for (let col = 0; col < grid; col++) {
      if (!bits[row][col]) continue;
      const isHighlight = (row + col) % 3 === 0;
      const x = gap + col * (cellSize + gap);
      const y = gap + row * (cellSize + gap);
      const r = cellSize * 0.15;
      rects += `<rect x="${x}" y="${y}" width="${cellSize}" height="${cellSize}" rx="${r}" fill="${isHighlight ? hi : fg}" />`;
    }
  }

  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${size} ${size}" width="${size}" height="${size}">${rects}</svg>`;
}
