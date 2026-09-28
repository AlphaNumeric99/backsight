/** Illustration for "no cameras yet": a camera with signal rings, in the accent tint. */
export function EmptyCamerasArt() {
  return (
    <svg width="168" height="120" viewBox="0 0 168 120" fill="none" aria-hidden>
      <defs>
        <linearGradient id="ec-body" x1="54" y1="34" x2="114" y2="92" gradientUnits="userSpaceOnUse">
          <stop stopColor="var(--surface)" />
          <stop offset="1" stopColor="var(--surface-3)" />
        </linearGradient>
        <radialGradient id="ec-lens" cx="0" cy="0" r="1" gradientUnits="userSpaceOnUse" gradientTransform="translate(80 58) rotate(90) scale(16)">
          <stop stopColor="#7C9BFF" />
          <stop offset="1" stopColor="#2A4FE0" />
        </radialGradient>
      </defs>
      <circle cx="84" cy="60" r="56" stroke="var(--brand)" strokeOpacity="0.1" strokeWidth="1.5" />
      <circle cx="84" cy="60" r="42" stroke="var(--brand)" strokeOpacity="0.16" strokeWidth="1.5" />
      <circle cx="84" cy="60" r="56" fill="var(--brand)" fillOpacity="0.04" />
      <rect x="50" y="34" width="68" height="52" rx="16" fill="url(#ec-body)" stroke="var(--border-strong)" />
      <circle cx="80" cy="60" r="16" fill="var(--video)" />
      <circle cx="80" cy="60" r="11" fill="url(#ec-lens)" />
      <circle cx="75.5" cy="55.5" r="3.2" fill="#fff" fillOpacity="0.8" />
      <circle cx="106" cy="45" r="3" fill="var(--success-dot)" />
      <rect x="72" y="86" width="16" height="10" rx="3" fill="var(--surface-3)" stroke="var(--border-strong)" />
      <rect x="62" y="95" width="36" height="6" rx="3" fill="var(--surface-3)" stroke="var(--border-strong)" />
      <path d="M130 30c5 4 8 10 8 16" stroke="var(--brand)" strokeWidth="2.5" strokeLinecap="round" />
      <path d="M136 22c8 6 12 15 12 24" stroke="var(--brand)" strokeOpacity="0.55" strokeWidth="2.5" strokeLinecap="round" />
      <path d="M24 72l3 3M24 78l3-3" stroke="var(--brand)" strokeOpacity="0.5" strokeWidth="2" strokeLinecap="round" />
      <circle cx="140" cy="86" r="2.5" fill="var(--brand)" fillOpacity="0.35" />
      <circle cx="30" cy="40" r="2" fill="var(--brand)" fillOpacity="0.35" />
    </svg>
  );
}
