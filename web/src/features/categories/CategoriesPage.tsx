import { useEffect, useState, type FormEvent } from "react";
import { FileText, Folder, FolderPlus, FolderTree, Pencil, Plus, Trash2, X } from "lucide-react";
import { createCategory, deleteCategory, renameCategory, type CategorySummary } from "../../lib/api";

type CategoriesPageProps = {
  categories: CategorySummary[];
  loading: boolean;
  onCatalogReload: () => void;
};

export function CategoriesPage({ categories, loading, onCatalogReload }: CategoriesPageProps) {
  const [rootName, setRootName] = useState("");
  const [editingId, setEditingId] = useState<string>();
  const [childParentId, setChildParentId] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const roots = categories.filter((category) => category.parentId === null);

  async function mutate(action: () => Promise<unknown>) {
    setBusy(true);
    setError(undefined);
    try {
      await action();
      setEditingId(undefined);
      setChildParentId(undefined);
      onCatalogReload();
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "The category could not be changed.");
    } finally {
      setBusy(false);
    }
  }

  function submitRoot(event: FormEvent) {
    event.preventDefault();
    const name = rootName.trim();
    if (!name) return;
    void mutate(async () => {
      await createCategory(name, null);
      setRootName("");
    });
  }

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
        <form className="category-create-root" onSubmit={submitRoot}>
          <label htmlFor="root-category-name">Create root category</label>
          <div><input disabled={busy} id="root-category-name" maxLength={120} onChange={(event) => setRootName(event.target.value)} placeholder="Category name" value={rootName} /><button disabled={busy || !rootName.trim()} type="submit"><Plus size={16} /> Create root</button></div>
        </form>
        {error && <p className="form-error category-error" role="alert">{error}</p>}
        {loading ? <div className="category-loading" /> : roots.length === 0 ? (
          <div className="library-empty">
            <FolderTree size={34} strokeWidth={1.3} />
            <h2>No categories yet</h2>
            <p>Create a root category above, or import a ZIP to build a tree from its folders.</p>
          </div>
        ) : (
          <div className="category-tree-large">
            {roots.map((root) => <CategoryBranch busy={busy} categories={categories} category={root} childParentId={childParentId} depth={0} editingId={editingId} key={root.id} mutate={mutate} onAddChild={setChildParentId} onEdit={setEditingId} />)}
          </div>
        )}
      </section>
    </div>
  );
}

type BranchProps = {
  busy: boolean;
  categories: CategorySummary[];
  category: CategorySummary;
  childParentId?: string;
  depth: number;
  editingId?: string;
  mutate: (action: () => Promise<unknown>) => Promise<void>;
  onAddChild: (id?: string) => void;
  onEdit: (id?: string) => void;
};

function CategoryBranch(props: BranchProps) {
  const { busy, categories, category, childParentId, depth, editingId, mutate, onAddChild, onEdit } = props;
  const [name, setName] = useState(category.name);
  const [childName, setChildName] = useState("");
  const children = categories.filter((candidate) => candidate.parentId === category.id);

  useEffect(() => setName(category.name), [category.name]);

  function submitRename(event: FormEvent) {
    event.preventDefault();
    const nextName = name.trim();
    if (nextName) void mutate(() => renameCategory(category.id, nextName));
  }

  function submitChild(event: FormEvent) {
    event.preventDefault();
    const nextName = childName.trim();
    if (!nextName) return;
    void mutate(async () => {
      await createCategory(nextName, category.id);
      setChildName("");
    });
  }

  function remove() {
    if (window.confirm(`Delete “${category.name}”? This cannot be undone.`)) {
      void mutate(() => deleteCategory(category.id));
    }
  }

  return <div className="category-branch">
    <div className="category-row" style={{ paddingLeft: 17 + depth * 27 }}>
      <Folder size={19} fill="currentColor" />
      {editingId === category.id ? <form className="category-inline-form" onSubmit={submitRename}><label className="sr-only" htmlFor={`rename-${category.id}`}>Rename {category.name}</label><input autoFocus disabled={busy} id={`rename-${category.id}`} maxLength={120} onChange={(event) => setName(event.target.value)} value={name} /><button aria-label={`Save ${category.name}`} disabled={busy || !name.trim()} type="submit">Save</button><button aria-label={`Cancel renaming ${category.name}`} onClick={() => { setName(category.name); onEdit(undefined); }} type="button"><X size={15} /></button></form> : <strong>{category.name}</strong>}
      <span className="category-count"><FileText size={13} /> {category.documentCount}</span>
      <span className="category-actions">
        <button aria-label={`Add child to ${category.name}`} disabled={busy} onClick={() => onAddChild(childParentId === category.id ? undefined : category.id)} type="button"><FolderPlus size={15} /></button>
        <button aria-label={`Rename ${category.name}`} disabled={busy} onClick={() => onEdit(category.id)} type="button"><Pencil size={14} /></button>
        <button aria-label={`Delete ${category.name}`} disabled={busy} onClick={remove} type="button"><Trash2 size={14} /></button>
      </span>
    </div>
    {childParentId === category.id && <form className="category-child-form" onSubmit={submitChild} style={{ paddingLeft: 49 + depth * 27 }}><label className="sr-only" htmlFor={`child-${category.id}`}>New child of {category.name}</label><input autoFocus disabled={busy} id={`child-${category.id}`} maxLength={120} onChange={(event) => setChildName(event.target.value)} placeholder={`New category in ${category.name}`} value={childName} /><button disabled={busy || !childName.trim()} type="submit">Add child</button><button aria-label="Cancel adding child" onClick={() => onAddChild(undefined)} type="button"><X size={15} /></button></form>}
    {children.map((child) => <CategoryBranch {...props} category={child} depth={depth + 1} key={child.id} />)}
  </div>;
}
