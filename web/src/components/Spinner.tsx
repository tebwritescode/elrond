/**
 * A busy indicator.
 *
 * Purely decorative: it is always accompanied by text, and the surrounding
 * control carries `aria-busy`. Announcing the spinner itself would add noise
 * without adding information.
 */
export function Spinner({ label }: { readonly label?: string }) {
  return (
    <>
      <span className="el-spinner" aria-hidden="true" />
      {label !== undefined && <span className="el-visually-hidden">{label}</span>}
    </>
  );
}
