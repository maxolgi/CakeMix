export interface GrMeterProps {
  reduction: number;       // gain reduction in dB (0 = no reduction, -20 = heavy)
  maxReduction?: number;   // scale: -maxReduction dB = full bar (default: -20)
  label?: string;          // optional label text (e.g. "GR")
  width?: number;          // bar width in px (default: 80)
  height?: number;         // bar height in px (default: 8)
}

// Gain reduction meter: horizontal bar reading left (heavy reduction) to
// right (0 dB / no reduction). Color steps: green (> -3 dB), yellow (-3..-10),
// red (< -10 dB). Updates reactively when `reduction` changes — no rAF loop.
export function GrMeter(props: GrMeterProps) {
  const maxRed = () => props.maxReduction ?? -20;
  const label = () => props.label ?? "GR";
  const width = () => props.width ?? 80;
  const height = () => props.height ?? 8;

  const fillPercent = () => {
    const mr = Math.abs(maxRed());
    if (mr <= 0) return 0;
    const r = Math.max(0, Math.abs(props.reduction));
    return Math.min(100, (r / mr) * 100);
  };

  const fillColor = () => {
    if (props.reduction > -3) return "#22c55e";
    if (props.reduction > -10) return "#eab308";
    return "#ef4444";
  };

  const valueText = () => props.reduction.toFixed(1);

  return (
    <div class="gr-meter-container" title={`${label()}: ${valueText()} dB`}>
      <span class="gr-meter-label">{label()}</span>
      <div
        class="gr-meter-bar-bg"
        style={{ width: `${width()}px`, height: `${height()}px` }}
      >
        <div
          class="gr-meter-bar-fill"
          style={{ width: `${fillPercent()}%`, "background-color": fillColor() }}
        />
      </div>
      <span class="gr-meter-value">{valueText()}</span>
    </div>
  );
}
