import { FileText, Folder, FolderTree } from "lucide-react";
import type { CategorySummary } from "../../lib/api";

type CategoriesPageProps = {
  categories: CategorySummary[];
  loading: boolean;
};

export function CategoriesPage({ categories, loading }: CategoriesPageProps) {
  const roots = categories.filter((category) => category.parentId === null);

  return (
    <div className="catalog-page">
      <header className="catalog-heading">
        <div>
          <p className="eyebrow">Information architecture</p>
          <h1>Categories</h1>
          <p>Every document has one primary home. Nested categories preserve the structure imported from your folders.</p>
        </div>
        <span>{categories.length} categor{categories.length === 1 ? "y" : "ies"}</span>
      </header>
      <section className="category-workspace">
        {loading ? <div className="category-loading" /> : roots.length === 0 ? (
          <div className="library-empty">
            <FolderTree size={34} strokeWidth={1.3} />
            <h2>No categories yet</h2>
            <p>Importing a ZIP creates this tree directly from its folders.</p>
          </div>
        ) : (
          <div className="category-tree-large">
            {roots.map((root) => <CategoryBranch categories={categories} category={root} depth={0} key={root.id} />)}
          </div>
        )}
      </section>
    </div>
  );
}

function CategoryBranch({ categories, category, depth }: { categories: CategorySummary[]; category: CategorySummary; depth: number }) {
  const children = categories.filter((candidate) => candidate.parentId === category.id);
  return (
    <div className="category-branch">
      <div className="category-row" style={{ paddingLeft: 17 + depth * 27 }}>
        <Folder size={19} fill="currentColor" />
        <strong>{category.name}</strong>
        <span><FileText size={13} /> {category.documentCount}</span>
      </div>
      {children.map((child) => <CategoryBranch categories={categories} category={child} depth={depth + 1} key={child.id} />)}
    </div>
  );
}
