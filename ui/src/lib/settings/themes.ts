// why: the picker's own catalog -- slugs must match themes.css's real
// `[data-theme="X"]` blocks exactly (sourced from tweakcn via BankkRoll/
// tweakcn-theme-picker's registry, dark variant of each; see themes.css's
// own doc). Categories mirror that project's own grouping, minus "Flora"
// (named in its README but not actually present in the registry -- no
// real data for it, so it's not offered here either).

export interface ThemeOption {
  slug: string;
  name: string;
}

export interface ThemeCategory {
  label: string;
  themes: ThemeOption[];
}

export const THEME_CATEGORIES: ThemeCategory[] = [
  {
    label: 'eqlp',
    themes: [{ slug: 'eqlp', name: 'Default (brass)' }],
  },
  {
    label: 'Minimal',
    themes: [
      { slug: 'default', name: 'Shadcn Default' },
      { slug: 'mocha-mousse', name: 'Mocha Mousse' },
      { slug: 'mono', name: 'Mono' },
      { slug: 'modern-minimal', name: 'Modern Minimal' },
      { slug: 'amber-minimal', name: 'Amber Minimal' },
      { slug: 'clean-slate', name: 'Clean Slate' },
    ],
  },
  {
    label: 'Colorful',
    themes: [
      { slug: 'catppuccin', name: 'Catppuccin' },
      { slug: 'bubblegum', name: 'Bubblegum' },
      { slug: 'nature', name: 'Nature' },
      { slug: 'ocean-breeze', name: 'Ocean Breeze' },
      { slug: 'sunset-horizon', name: 'Sunset Horizon' },
      { slug: 'pastel-dreams', name: 'Pastel Dreams' },
      { slug: 'perpetuity', name: 'Perpetuity' },
      { slug: 'tangerine', name: 'Tangerine' },
      { slug: 'solar-dusk', name: 'Solar Dusk' },
      { slug: 'candyland', name: 'Candyland' },
      { slug: 'northern-lights', name: 'Northern Lights' },
    ],
  },
  {
    label: 'Branded',
    themes: [
      { slug: 'claude', name: 'Claude' },
      { slug: 'vercel', name: 'Vercel' },
      { slug: 't3-chat', name: 'T3 Chat' },
      { slug: 'twitter', name: 'Twitter' },
      { slug: 'bold-tech', name: 'Bold Tech' },
      { slug: 'supabase', name: 'Supabase' },
      { slug: 'twitch', name: 'Twitch' },
      { slug: 'kick', name: 'Kick' },
      { slug: 'spotify', name: 'Spotify' },
      { slug: 'stripe', name: 'Stripe' },
      { slug: 'github', name: 'GitHub' },
    ],
  },
  {
    label: 'Creative',
    themes: [
      { slug: 'cyberpunk', name: 'Cyberpunk' },
      { slug: 'neo-brutalism', name: 'Neo Brutalism' },
      { slug: 'doom-64', name: 'Doom 64' },
      { slug: 'kodama-grove', name: 'Kodama Grove' },
      { slug: 'quantum-rose', name: 'Quantum Rose' },
      { slug: 'elegant-luxury', name: 'Elegant Luxury' },
      { slug: 'claymorphism', name: 'Claymorphism' },
      { slug: 'retro-arcade', name: 'Retro Arcade' },
      { slug: 'vintage-paper', name: 'Vintage Paper' },
      { slug: 'windows98', name: 'Windows 98' },
    ],
  },
  {
    label: 'Dark',
    themes: [
      { slug: 'cosmic-night', name: 'Cosmic Night' },
      { slug: 'midnight-bloom', name: 'Midnight Bloom' },
      { slug: 'graphite', name: 'Graphite' },
      { slug: 'caffeine', name: 'Caffeine' },
      { slug: 'starry-night', name: 'Starry Night' },
    ],
  },
];

export const ALL_THEMES: ThemeOption[] = THEME_CATEGORIES.flatMap((c) => c.themes);

export function themeName(slug: string): string {
  return ALL_THEMES.find((t) => t.slug === slug)?.name ?? slug;
}
// why: a small representative slice of each theme's own real palette
// (background, primary, accent, destructive) for the picker's own swatch
// dots -- extracted directly from themes.css's real values, not
// hand-picked, so a swatch never drifts from what selecting the theme
// actually applies. oklch()/hex strings, used directly as CSS
// background-color -- no parsing needed, browsers render both natively.
export const THEME_SWATCHES: Record<string, string[]> = {
  'eqlp': ['#15171b', '#c9a15a', '#262a31', '#e0616f'],
  'amber-minimal': ['oklch(0.2 0 0)', 'oklch(0.77 0.16 70.08)', 'oklch(0.47 0.12 46.2)', 'oklch(0.64 0.21 25.33)'],
  'bold-tech': ['oklch(0.21 0.04 265.75)', 'oklch(0.61 0.22 292.72)', 'oklch(0.46 0.21 277.02)', 'oklch(0.64 0.21 25.33)'],
  'bubblegum': ['oklch(0.25 0.03 234.16)', 'oklch(0.92 0.08 87.67)', 'oklch(0.67 0.1 356.98)', 'oklch(0.67 0.18 350.36)'],
  'caffeine': ['oklch(0.18 0 0)', 'oklch(0.92 0.05 66.17)', 'oklch(0.29 0 0)', 'oklch(0.63 0.19 33.34)'],
  'candyland': ['oklch(0.23 0.01 264.29)', 'oklch(0.8 0.14 349.23)', 'oklch(0.81 0.08 225.75)', 'oklch(0.64 0.21 25.33)'],
  'catppuccin': ['oklch(0.22 0.03 284.06)', 'oklch(0.79 0.12 304.77)', 'oklch(0.85 0.08 210.25)', 'oklch(0.76 0.13 2.76)'],
  'claude': ['oklch(0.27 0 106.64)', 'oklch(0.67 0.13 38.76)', 'oklch(0.21 0.01 95.42)', 'oklch(0.64 0.21 25.33)'],
  'claymorphism': ['oklch(0.22 0.01 67.44)', 'oklch(0.68 0.16 276.93)', 'oklch(0.39 0.01 59.47)', 'oklch(0.64 0.21 25.33)'],
  'clean-slate': ['oklch(0.21 0.04 265.75)', 'oklch(0.68 0.16 276.93)', 'oklch(0.37 0.03 259.73)', 'oklch(0.64 0.21 25.33)'],
  'cosmic-night': ['oklch(0.17 0.02 283.8)', 'oklch(0.72 0.16 290.4)', 'oklch(0.34 0.08 280.97)', 'oklch(0.69 0.21 14.99)'],
  'cyberpunk': ['oklch(0.16 0.04 281.83)', 'oklch(0.67 0.29 341.41)', 'oklch(0.89 0.17 171.27)', 'oklch(0.65 0.23 34.04)'],
  'default': ['oklch(0.1450 0 0)', 'oklch(0.9220 0 0)', 'oklch(0.3710 0 0)', 'oklch(0.7040 0.1910 22.2160)'],
  'doom-64': ['oklch(0.22 0 0)', 'oklch(0.61 0.21 27.03)', 'oklch(0.75 0.12 244.75)', 'oklch(0.78 0.17 68.09)'],
  'elegant-luxury': ['oklch(0.22 0.01 56.04)', 'oklch(0.51 0.19 27.52)', 'oklch(0.56 0.15 49)', 'oklch(0.64 0.21 25.33)'],
  'github': ['oklch(0.15 0.012 250)', 'oklch(0.55 0.15 145)', 'oklch(0.57 0.18 260)', 'oklch(0.60 0.18 25)'],
  'graphite': ['oklch(0.22 0 0)', 'oklch(0.71 0 0)', 'oklch(0.37 0 0)', 'oklch(0.66 0.15 22.17)'],
  'kick': ['oklch(0.12 0 0)', 'oklch(0.85 0.3 128)', 'oklch(0.85 0.3 128)', 'oklch(0.6 0.2 25)'],
  'kodama-grove': ['oklch(0.33 0.02 88.07)', 'oklch(0.68 0.06 132.45)', 'oklch(0.65 0.07 90.76)', 'oklch(0.63 0.08 31.3)'],
  'midnight-bloom': ['oklch(0.23 0.01 264.29)', 'oklch(0.57 0.2 283.08)', 'oklch(0.67 0.14 261.34)', 'oklch(0.64 0.21 25.33)'],
  'mocha-mousse': ['oklch(0.27 0.01 48.18)', 'oklch(0.73 0.05 52.33)', 'oklch(0.75 0.04 80.55)', 'oklch(0.69 0.14 21.46)'],
  'modern-minimal': ['oklch(0.2 0 0)', 'oklch(0.62 0.19 259.81)', 'oklch(0.38 0.14 265.52)', 'oklch(0.64 0.21 25.33)'],
  'mono': ['oklch(0.14 0 0)', 'oklch(0.56 0 0)', 'oklch(0.37 0 0)', 'oklch(0.7 0.19 22.23)'],
  'nature': ['oklch(0.27 0.03 150.77)', 'oklch(0.67 0.16 144.21)', 'oklch(0.58 0.14 144.18)', 'oklch(0.54 0.19 26.72)'],
  'neo-brutalism': ['oklch(0 0 0)', 'oklch(0.7 0.19 23.19)', 'oklch(0.68 0.18 252.26)', 'oklch(1 0 0)'],
  'northern-lights': ['oklch(0.23 0.01 264.29)', 'oklch(0.65 0.15 150.31)', 'oklch(0.67 0.14 261.34)', 'oklch(0.64 0.21 25.33)'],
  'ocean-breeze': ['oklch(0.21 0.04 265.75)', 'oklch(0.77 0.15 163.22)', 'oklch(0.37 0.03 259.73)', 'oklch(0.64 0.21 25.33)'],
  'pastel-dreams': ['oklch(0.22 0.01 56.04)', 'oklch(0.79 0.12 295.75)', 'oklch(0.39 0.05 304.64)', 'oklch(0.81 0.1 19.57)'],
  'perpetuity': ['oklch(0.21 0.02 224.45)', 'oklch(0.85 0.13 195.04)', 'oklch(0.38 0.06 216.5)', 'oklch(0.62 0.21 25.81)'],
  'quantum-rose': ['oklch(0.18 0.05 313.72)', 'oklch(0.75 0.23 332.02)', 'oklch(0.36 0.12 325.77)', 'oklch(0.65 0.24 7.17)'],
  'retro-arcade': ['oklch(0.27 0.05 219.82)', 'oklch(0.59 0.2 355.89)', 'oklch(0.58 0.17 39.5)', 'oklch(0.59 0.21 27.12)'],
  'solar-dusk': ['oklch(0.22 0.01 56.04)', 'oklch(0.7 0.19 47.6)', 'oklch(0.36 0.05 229.32)', 'oklch(0.58 0.22 27.33)'],
  'spotify': ['oklch(0.145 0 0)', 'oklch(0.64 0.18 152)', 'oklch(0.72 0.20 150)', 'oklch(0.60 0.20 25)'],
  'starry-night': ['oklch(0.22 0.02 275.84)', 'oklch(0.48 0.12 263.38)', 'oklch(0.85 0.05 264.78)', 'oklch(0.53 0.12 357.11)'],
  'stripe': ['oklch(0.22 0.05 250)', 'oklch(0.6 0.23 285)', 'oklch(0.82 0.14 200)', 'oklch(0.6 0.2 25)'],
  'sunset-horizon': ['oklch(0.26 0.02 352.4)', 'oklch(0.74 0.16 34.71)', 'oklch(0.83 0.11 58)', 'oklch(0.61 0.21 22.24)'],
  'supabase': ['oklch(0.18 0 0)', 'oklch(0.44 0.1 156.76)', 'oklch(0.31 0 0)', 'oklch(0.31 0.09 29.79)'],
  't3-chat': ['oklch(0.24 0.02 307.53)', 'oklch(0.46 0.19 4.1)', 'oklch(0.36 0.05 308.49)', 'oklch(0.23 0.05 12.61)'],
  'tangerine': ['oklch(0.26 0.03 262.67)', 'oklch(0.64 0.17 36.44)', 'oklch(0.34 0.06 267.59)', 'oklch(0.64 0.21 25.33)'],
  'twitch': ['oklch(0.14 0.005 285)', 'oklch(0.54 0.27 290)', 'oklch(0.72 0.18 290)', 'oklch(0.6 0.2 25)'],
  'twitter': ['oklch(0 0 0)', 'oklch(0.67 0.16 245.01)', 'oklch(0.19 0.03 242.55)', 'oklch(0.62 0.24 25.77)'],
  'vercel': ['oklch(0 0 0)', 'oklch(1 0 0)', 'oklch(0.32 0 0)', 'oklch(0.69 0.2 23.91)'],
  'vintage-paper': ['oklch(0.27 0.01 57.65)', 'oklch(0.73 0.06 66.7)', 'oklch(0.42 0.03 56.34)', 'oklch(0.55 0.14 32.91)'],
  'windows98': ['oklch(0.5431 0.0927 194.7689)', 'oklch(0.2711 0.1879 264.052)', 'oklch(0.2711 0.1879 264.052)', 'oklch(0.628 0.2577 29.2339)'],
};
