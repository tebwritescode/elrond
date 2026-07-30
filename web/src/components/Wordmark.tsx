import { IconLibrary } from './Icon';

/**
 * The Elrond wordmark.
 *
 * The version is rendered next to it because two implementations of this product
 * may be running side by side; being able to see which one you are looking at
 * matters more than a clean header.
 */
export function Wordmark({ version }: { readonly version?: string | undefined }) {
  return (
    <span className="el-wordmark">
      <span className="el-wordmark__mark">
        <IconLibrary size={20} />
      </span>
      <span>Elrond</span>
      {version !== undefined && (
        <span className="el-eyebrow" style={{ marginLeft: '0.125rem' }}>
          {version}
        </span>
      )}
    </span>
  );
}
