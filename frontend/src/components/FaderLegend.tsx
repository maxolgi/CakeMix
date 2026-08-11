// FaderLegend.tsx — SVG dB scale for the fader, ported from Eyevinn SliderLegend
export function FaderLegend() {
  const ticks = [
    { db: "+10", y: 4, major: true },
    { db: "+5", y: 36 },
    { db: "0", y: 68, major: true },
    { db: "-5", y: 100 },
    { db: "-10", y: 132 },
    { db: "-20", y: 164 },
    { db: "-30", y: 196 },
    { db: "-40", y: 212 },
    { db: "-50", y: 228 },
    { db: "-60", y: 244 },
    { db: "-∞", y: 260 },
  ];
  return (
    <svg class="fader-legend-svg" viewBox="0 0 67 280" xmlns="http://www.w3.org/2000/svg">
      {ticks.map((t) => (
        <>
          <line
            x1="22" y1={t.y} x2="67" y2={t.y}
            stroke={t.major ? "#aaa" : "#555"}
            stroke-width={t.major ? 1.5 : 1}
          />
          <text
            x="0" y={t.y + 4}
            font-size="9"
            fill={t.major ? "#aaa" : "#666"}
            font-family="monospace"
          >{t.db}</text>
        </>
      ))}
    </svg>
  );
}
