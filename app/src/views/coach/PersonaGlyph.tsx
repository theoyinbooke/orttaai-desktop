// Each coach's mark (Mac CoachPersonaGlyph): a shape on an amber disc, so
// coaches read apart without faces or extra colours.
import type { PersonaId } from "../../lib/coach";

export default function PersonaGlyph({ persona, size = 44, color = "var(--accent)" }: { persona: PersonaId; size?: number; color?: string }) {
  const line = Math.max(1.4, size / 26) * (100 / size);
  const c = 50;
  return (
    <svg width={size} height={size} viewBox="0 0 100 100" aria-hidden="true" style={{ flex: "none" }}>
      <circle cx={c} cy={c} r={50} fill={color} fillOpacity={0.16} />
      {persona === "mentor" && (
        <>
          <circle cx={c} cy={c} r={27} fill="none" stroke={color} strokeWidth={line} />
          <circle cx={c} cy={c} r={10} fill={color} />
        </>
      )}
      {persona === "editor" && (
        <>
          <rect x={29} y={29} width={42} height={42} rx={8} fill="none" stroke={color} strokeWidth={line} />
          <rect x={40} y={c - line / 2} width={20} height={line} rx={line / 2} fill={color} />
        </>
      )}
      {persona === "socratic" && (
        <path
          d="M38 38 C38 26 62 26 62 38 C62 47 50 47 50 57 M50 68 L50 69"
          fill="none"
          stroke={color}
          strokeWidth={line * 1.15}
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      )}
      {persona === "sergeant" && (
        <path d="M50 28 L73 68 L27 68 Z" fill="none" stroke={color} strokeWidth={line} strokeLinejoin="round" />
      )}
      {persona === "peer" && (
        <>
          <circle cx={40} cy={c} r={15} fill="none" stroke={color} strokeWidth={line} />
          <circle cx={60} cy={c} r={15} fill="none" stroke={color} strokeWidth={line} />
        </>
      )}
    </svg>
  );
}
