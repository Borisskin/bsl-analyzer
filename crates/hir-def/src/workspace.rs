use rustc_hash::FxHashMap;
use std::sync::Arc;
use tracing::debug;
use vfs::{FileId, FileSet};

use crate::{DefDatabase, MethodSymbol, ModuleId, Name};

pub fn is_bsl_source(file_set: &FileSet, file_id: FileId) -> bool {
    let Some(vfs_path) = file_set.path_for_file(&file_id) else {
        return false;
    };
    project_model::is_bsl_source_path(vfs_path.as_path())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommonModuleInfo {
    pub file_id: FileId,

    pub module_id: ModuleId,

    pub methods: Vec<MethodSymbol>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkspaceSymbols {
    pub common_modules: FxHashMap<Name, CommonModuleInfo>,
}

/// Approximate live heap bytes for Salsa's `memory_usage` report: the
/// `common_modules` table, each module name's `Name` payload, and each
/// module's `Vec<MethodSymbol>` — summing each method's name, params, and
/// annotations. `docs`/`return_type_ref`/param `type_ref` payloads are not
/// followed (mirrors `symbol_tree_heap`'s convention), a mild undercount.
/// New heap-owning fields must be added here too.
pub(crate) fn workspace_symbols_heap(v: &Arc<WorkspaceSymbols>) -> usize {
    use crate::heap_estimate::{map_table_bytes, name_bytes, vec_bytes};
    use crate::item_tree::Annotation;
    use crate::ParamSymbol;

    let s = &**v;
    let mut bytes = map_table_bytes::<Name, CommonModuleInfo>(s.common_modules.len());
    for (name, info) in &s.common_modules {
        bytes += name_bytes(name);
        bytes += vec_bytes::<MethodSymbol>(info.methods.len());
        for method in &info.methods {
            bytes += name_bytes(&method.name);
            bytes += vec_bytes::<ParamSymbol>(method.params.len());
            for param in &method.params {
                bytes += name_bytes(&param.name);
            }
            bytes += vec_bytes::<Annotation>(method.annotations.len());
        }
    }
    bytes
}

pub fn workspace_symbols(db: &dyn DefDatabase, files: &[FileId]) -> WorkspaceSymbols {
    let mut common_modules = FxHashMap::default();

    debug!(file_count = files.len(), "Building workspace symbol index");

    for &file_id in files {
        let module_id = ModuleId::new(file_id);
        let symbol_tree = db.symbol_tree(module_id);

        let methods: Vec<_> = symbol_tree.methods().cloned().collect();
        if !methods.is_empty() {
            let module_name = extract_module_name_from_file(db, file_id).unwrap_or_else(|| {
                debug!(
                    file_id = ?file_id,
                    "Could not extract module name, using FileId"
                );
                Name::new(&format!("Module{}", file_id.0))
            });

            debug!(
                file_id = ?file_id,
                module_name = %module_name,
                method_count = methods.len(),
                "Indexed module"
            );

            common_modules.insert(module_name, CommonModuleInfo { file_id, module_id, methods });
        }
    }

    debug!(common_module_count = common_modules.len(), "Workspace symbol index built");

    WorkspaceSymbols { common_modules }
}

fn extract_module_name_from_file(db: &dyn DefDatabase, file_id: FileId) -> Option<Name> {
    let file_source_root_input = db.file_source_root_input(file_id);
    let source_root_id = file_source_root_input.source_root_id(db);
    let source_root_input = db.source_root_input(source_root_id);
    let source_root = source_root_input.root(db);
    let file_set = source_root.file_set();

    let vfs_path = file_set.path_for_file(&file_id)?;
    let path_str = vfs_path.as_path().to_string_lossy();
    module_name_from_path(&path_str)
}

/// Derive a module name from a file path: the segment after `CommonModules/`
/// (localized `ОбщиеМодули/`), else the `.bsl` file stem.
fn module_name_from_path(path: &str) -> Option<Name> {
    // The VFS stores native paths; on Windows those use `\`. The segment matching
    // below keys on `/`, so normalize first — otherwise `CommonModules/` is never
    // found and the filename split yields the whole path as the module name.
    let path_str = path.replace('\\', "/");

    debug!(path = %path_str, "Extracting module name from path");

    if let Some(pos) = path_str.find("CommonModules/").or_else(|| path_str.find("ОбщиеМодули/"))
    {
        let after_common = if path_str[pos..].starts_with("CommonModules/") {
            &path_str[pos + "CommonModules/".len()..]
        } else {
            &path_str[pos + "ОбщиеМодули/".len()..]
        };

        if let Some(slash_pos) = after_common.find('/') {
            let module_name = &after_common[..slash_pos];
            debug!(module_name, "Extracted from CommonModules path");
            return Some(Name::new(module_name));
        }
    }

    if let Some(filename) = path_str.rsplit('/').next() {
        if let Some(name_without_ext) = filename.strip_suffix(".bsl") {
            debug!(module_name = name_without_ext, "Extracted from filename");
            return Some(Name::new(name_without_ext));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use vfs::{FileSet, VfsPath};

    #[test]
    fn test_common_module_info_creation() {
        let file_id = FileId(0);
        let module_id = ModuleId::new(file_id);
        let methods = vec![];

        let info = CommonModuleInfo { file_id, module_id, methods };

        assert_eq!(info.file_id, file_id);
        assert_eq!(info.module_id, module_id);
        assert_eq!(info.methods.len(), 0);
    }

    fn make_file_set(entries: &[(u32, &str)]) -> FileSet {
        let mut fs = FileSet::new();
        for (id, path) in entries {
            fs.insert(FileId(*id), VfsPath::new(std::path::PathBuf::from(path)));
        }
        fs
    }

    #[test]
    fn is_bsl_source_accepts_bsl_extension() {
        let fs = make_file_set(&[
            (1, "/ws/CommonModules/MyModule/Ext/Module.bsl"),
            (2, "/ws/Documents/X/Ext/ObjectModule.bsl"),
        ]);
        assert!(is_bsl_source(&fs, FileId(1)));
        assert!(is_bsl_source(&fs, FileId(2)));
    }

    #[test]
    fn is_bsl_source_rejects_non_bsl_files() {
        let fs = make_file_set(&[
            (10, "/ws/Roles/R/Ext/Rights.xml"),
            (11, "/ws/InformationRegisters/X/Templates/T/Ext/Template.xml"),
            (12, "/ws/Configuration.xml"),
            (13, "/ws/README.md"),
            (14, "/ws/notes.txt"),
            (15, "/ws/NoExtensionAtAll"),
        ]);
        for id in 10..=15 {
            assert!(!is_bsl_source(&fs, FileId(id)), "FileId({id}) should be filtered out");
        }
    }

    #[test]
    fn is_bsl_source_is_case_insensitive() {
        let fs = make_file_set(&[
            (20, "/ws/CommonModules/A/Ext/Module.BSL"),
            (21, "/ws/CommonModules/B/Ext/Module.Bsl"),
        ]);
        assert!(is_bsl_source(&fs, FileId(20)));
        assert!(is_bsl_source(&fs, FileId(21)));
    }

    #[test]
    fn is_bsl_source_rejects_unknown_file_id() {
        let fs = make_file_set(&[(1, "/ws/a.bsl")]);
        assert!(!is_bsl_source(&fs, FileId(999)));
    }

    #[test]
    fn module_name_from_forward_slash_paths() {
        assert_eq!(
            module_name_from_path("/ws/src/cf/CommonModules/ОбщегоНазначения/Ext/Module.bsl"),
            Some(Name::new("ОбщегоНазначения"))
        );
        assert_eq!(
            module_name_from_path("/ws/Documents/Заказ/Ext/ObjectModule.bsl"),
            Some(Name::new("ObjectModule"))
        );
    }

    #[test]
    fn module_name_from_windows_backslash_paths() {
        assert_eq!(
            module_name_from_path(r"C:\ws\src\cf\CommonModules\ОбщегоНазначения\Ext\Module.bsl"),
            Some(Name::new("ОбщегоНазначения"))
        );
        assert_eq!(
            module_name_from_path(r"C:\ws\Documents\Заказ\Ext\ObjectModule.bsl"),
            Some(Name::new("ObjectModule"))
        );
    }
}
