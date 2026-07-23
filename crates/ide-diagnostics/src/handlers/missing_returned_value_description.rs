use crate::define_metadata;
use crate::metadata::*;
use crate::{Diagnostic, DiagnosticCode, DiagnosticsContext};
use hir::ModItem;
use ide_db::TextRange;

pub const METADATA: DiagnosticMetadata = define_metadata! {
    diagnostic_type: DiagnosticType::CodeSmell,
    severity: DiagnosticSeverityLevel::Major,
    scope: DiagnosticScope::All,
    modules: &[],
    minutes_to_fix: 5,
    activated_by_default: true,
    compatibility_mode: DiagnosticCompatibilityMode::Undefined,
    tags: &[MetadataTag::Standard, MetadataTag::Badpractice],
    can_locate_on_project: false,
    extra_min_for_complexity: 0.0,
    lsp_severity_override: "",
};

pub fn check(ctx: &DiagnosticsContext) -> Vec<Diagnostic> {
    let code = DiagnosticCode::MissingReturnedValueDescription;

    if ctx.is_disabled_with_metadata(code) {
        return Vec::new();
    }

    let mut diagnostics = Vec::new();

    let module_data = ctx.module_data();

    for method_id in &module_data.procedures {
        if let Some(diag) = check_procedure_hir(ctx, *method_id, code) {
            diagnostics.push(diag);
        }
    }

    for method_id in &module_data.functions {
        if let Some(diag) = check_function_hir(ctx, *method_id, code) {
            diagnostics.push(diag);
        }
    }

    diagnostics
}

fn check_function_hir(
    ctx: &DiagnosticsContext,
    method_id: hir::MethodId,
    code: DiagnosticCode,
) -> Option<Diagnostic> {
    let tree = ctx.item_tree();

    let func_info =
        tree.top_level_items().get(method_id.local_id as usize).and_then(|item| match item {
            ModItem::Function(func_idx) => {
                let func = tree.function(*func_idx);
                Some((func.is_export, func.name_range))
            }
            _ => None,
        });

    let (is_export, name_range) = func_info?;

    if !is_export {
        return None;
    }

    let docs = match ctx.method_docs(method_id) {
        Some(docs) => docs,
        None => {
            if !ctx.is_disabled_with_metadata(DiagnosticCode::PublicMethodsDescription) {
                return None;
            }
            return Some(create_diagnostic(
                name_range,
                "Добавьте описание возвращаемого значения функции",
                code,
                ctx,
            ));
        }
    };

    if docs.is_hyperlink() {
        return None;
    }

    if docs.returned_value.is_empty() {
        // bsl-ls suppresses this case when the doc contains any `см.` link
        // (`MethodDescription.getLinks()`), even one inside a parameter
        // description. We deliberately do not: an inline link elsewhere in
        // the doc does not document the return value.
        return Some(create_diagnostic(
            name_range,
            "Добавьте описание возвращаемого значения функции",
            code,
            ctx,
        ));
    }

    let allow_short = ctx
        .config
        .get_bool(
            DiagnosticCode::MissingReturnedValueDescription,
            "allowShortDescriptionReturnValues",
        )
        .unwrap_or(true);

    if !allow_short {
        let types_without_desc: Vec<&str> = docs
            .returned_value
            .iter()
            .filter_map(|type_doc| {
                if type_doc.description.is_none() && type_doc.parameters.is_empty() {
                    Some(type_doc.name.as_str())
                } else {
                    None
                }
            })
            .collect();

        if !types_without_desc.is_empty() {
            let types_list = types_without_desc.join(", ");
            let message = format!(
                "Необходимо добавить описание типов \"{}\" возвращаемого значения",
                types_list
            );
            return Some(create_diagnostic(name_range, &message, code, ctx));
        }
    }

    None
}

fn check_procedure_hir(
    ctx: &DiagnosticsContext,
    method_id: hir::MethodId,
    code: DiagnosticCode,
) -> Option<Diagnostic> {
    let tree = ctx.item_tree();

    let name_range =
        tree.top_level_items().get(method_id.local_id as usize).and_then(|item| match item {
            ModItem::Procedure(proc_idx) => Some(tree.procedure(*proc_idx).name_range),
            _ => None,
        })?;

    let docs = ctx.method_docs(method_id)?;

    if !docs.returned_value.is_empty() {
        return Some(create_diagnostic(
            name_range,
            "Удалите описание возвращаемого значения для процедуры",
            code,
            ctx,
        ));
    }

    None
}

fn create_diagnostic(
    range: TextRange,
    message: &str,
    code: DiagnosticCode,
    ctx: &DiagnosticsContext,
) -> Diagnostic {
    Diagnostic {
        code,
        message: message.to_string(),
        severity: ctx.severity(code),
        range,
        tags: ctx.tags(code),
        fixes: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::check;
    use crate::test_utils::{
        check_ast_diagnostic, check_ast_diagnostic_with_config, check_diagnostics_snapshot_for,
        format_diags,
    };
    use crate::{DiagnosticCode, DiagnosticsConfig};
    use expect_test::expect;
    #[test]
    fn test_function_without_comments() {
        let code = "Функция Example()\nКонецФункции";
        check_diagnostics_snapshot_for(
            code,
            DiagnosticCode::MissingReturnedValueDescription,
            expect![[r#""#]],
        );
    }

    #[test]
    fn test_export_function_without_comments() {
        let code = "Функция Example() Экспорт\nКонецФункции";
        check_diagnostics_snapshot_for(
            code,
            DiagnosticCode::MissingReturnedValueDescription,
            expect![[r#""#]],
        );
    }

    #[test]
    fn test_export_function_without_comments_pmd_disabled() {
        let code = "Функция Example() Экспорт\nКонецФункции";
        let mut config = DiagnosticsConfig::default();
        config.disabled.push(DiagnosticCode::PublicMethodsDescription);
        let diagnostics = check_ast_diagnostic_with_config(code, config, check);
        expect![[r#"
            MissingReturnedValueDescription @ 1:9..1:16
              message: Добавьте описание возвращаемого значения функции
              severity: Warning"#]]
        .assert_eq(&format_diags(code, &diagnostics));
    }

    #[test]
    fn test_function_with_description_no_return() {
        let code = "// Описание вроде\nФункция Example() Экспорт\nКонецФункции";
        check_diagnostics_snapshot_for(
            code,
            DiagnosticCode::MissingReturnedValueDescription,
            expect![[r#"
                MissingReturnedValueDescription @ 2:9..2:16
                  message: Добавьте описание возвращаемого значения функции
                  severity: Warning"#]],
        );
    }

    #[test]
    fn test_function_with_empty_return_block() {
        let code =
            "// Описание вроде\n// Возвращаемое значение:\nФункция Example() Экспорт\nКонецФункции";
        check_diagnostics_snapshot_for(
            code,
            DiagnosticCode::MissingReturnedValueDescription,
            expect![[r#"
                MissingReturnedValueDescription @ 3:9..3:16
                  message: Добавьте описание возвращаемого значения функции
                  severity: Warning"#]],
        );
    }

    #[test]
    fn test_function_with_complete_description() {
        let code = "// Описание вроде\n// Возвращаемое значение:\n// Строка - строка типа\nФункция Example()\nКонецФункции";
        check_diagnostics_snapshot_for(
            code,
            DiagnosticCode::MissingReturnedValueDescription,
            expect![[r#""#]],
        );
    }

    #[test]
    fn test_procedure_with_return_description() {
        let code =
            "// Описание вроде\n// Возвращаемое значение:\n// Строка - строка типа\nПроцедура Example()\nКонецПроцедуры";
        check_diagnostics_snapshot_for(
            code,
            DiagnosticCode::MissingReturnedValueDescription,
            expect![[r#"
                MissingReturnedValueDescription @ 4:11..4:18
                  message: Удалите описание возвращаемого значения для процедуры
                  severity: Warning"#]],
        );
    }

    #[test]
    fn test_procedure_without_return() {
        let code = "// Описание вроде\nПроцедура Example()\nКонецПроцедуры";
        check_diagnostics_snapshot_for(
            code,
            DiagnosticCode::MissingReturnedValueDescription,
            expect![[r#""#]],
        );
    }

    #[test]
    fn test_function_with_type_no_description_default_mode() {
        let code = "// Описание вроде\n// Возвращаемое значение:\n// Строка\nФункция Example()\nКонецФункции";
        check_diagnostics_snapshot_for(
            code,
            DiagnosticCode::MissingReturnedValueDescription,
            expect![[r#""#]],
        );
    }

    #[test]
    fn test_function_with_type_no_description_strict_mode() {
        let code = "// Описание вроде\n// Возвращаемое значение:\n// Строка\nФункция Example() Экспорт\nКонецФункции";

        let mut config = DiagnosticsConfig::default();
        config.parameters.insert(
            DiagnosticCode::MissingReturnedValueDescription,
            serde_json::json!({"allowShortDescriptionReturnValues": false}),
        );

        let diagnostics = check_ast_diagnostic_with_config(code, config, check);
        expect![[r#"
            MissingReturnedValueDescription @ 4:9..4:16
              message: Необходимо добавить описание типов "Строка" возвращаемого значения
              severity: Warning"#]]
        .assert_eq(&format_diags(code, &diagnostics));
    }

    #[test]
    fn test_function_with_hyperlink_reference() {
        let code = "// См. Пример7()\nФункция Example()\nКонецФункции";
        check_diagnostics_snapshot_for(
            code,
            DiagnosticCode::MissingReturnedValueDescription,
            expect![[r#""#]],
        );
    }

    #[test]
    fn test_hyperlink_only_delegated_doc_snapshot() {
        check_diagnostics_snapshot_for(
            "// См. ДругойМетод()\nФункция Example() Экспорт\nКонецФункции",
            DiagnosticCode::MissingReturnedValueDescription,
            expect![[r#""#]],
        );
    }

    #[test]
    fn test_empty_doc_body_regression_guard_snapshot() {
        check_diagnostics_snapshot_for(
            "// Параметры:\n//\nФункция Example() Экспорт\nКонецФункции",
            DiagnosticCode::MissingReturnedValueDescription,
            expect![[r#"
                MissingReturnedValueDescription @ 3:9..3:16
                  message: Добавьте описание возвращаемого значения функции
                  severity: Warning"#]],
        );
    }

    #[test]
    fn test_pmd_enabled_context_no_double_diag_snapshot() {
        check_diagnostics_snapshot_for(
            r#"#Область ПрограммныйИнтерфейс

Функция Example() Экспорт
КонецФункции

#КонецОбласти"#,
            DiagnosticCode::MissingReturnedValueDescription,
            expect![[r#""#]],
        );
    }

    #[test]
    fn test_function_with_multiple_types_no_description_strict() {
        let code = "// Описание вроде\n// Возвращаемое значение:\n// - Строка\n// - булево\nФункция Example() Экспорт\nКонецФункции";

        let mut config = DiagnosticsConfig::default();
        config.parameters.insert(
            DiagnosticCode::MissingReturnedValueDescription,
            serde_json::json!({"allowShortDescriptionReturnValues": false}),
        );

        let diagnostics = check_ast_diagnostic_with_config(code, config, check);
        expect![[r#"
            MissingReturnedValueDescription @ 5:9..5:16
              message: Необходимо добавить описание типов "Строка, булево" возвращаемого значения
              severity: Warning"#]]
        .assert_eq(&format_diags(code, &diagnostics));
    }

    #[test]
    fn test_english_keywords() {
        let code =
            "// Description\n// Returns:\n// String - result\nFunction Example()\nEndFunction";
        check_diagnostics_snapshot_for(
            code,
            DiagnosticCode::MissingReturnedValueDescription,
            expect![[r#""#]],
        );
    }

    #[test]
    fn test_structure_with_nested_fields() {
        let code = r#"// Возвращает структуру с доступными публикациями HTTP-сервисов ERP.
//
// Возвращаемое значение:
//   Структура - Структура с ключами-названиями сервисов и значениями-URL путями к публикациям:
//     * ПОЗК - Строка - Публикация для работы с производственными заказами.
//     * ДанныеДО - Строка - Публикация для получения данных документооборота.
//     * ДанныеДООтветственный - Строка - Публикация для получения данных об ответственных.
//     * Рецептура - Строка - Публикация для работы с рецептурами.
//
Функция ПубликацииERP() Экспорт
    Структура = Новый Структура;
    Структура.Вставить("ПОЗК", "/hs/pozk/getdirection");
    Структура.Вставить("ДанныеДО", "/hs/dodata/statusdocument");
    Структура.Вставить("ДанныеДООтветственный", "/hs/dodata/responsible");
    Структура.Вставить("Рецептура", "/hs/recipe/changestatus");
    Возврат Структура;
КонецФункции"#;
        check_diagnostics_snapshot_for(
            code,
            DiagnosticCode::MissingReturnedValueDescription,
            expect![[r#""#]],
        );
    }

    #[test]
    fn test_diagnostic_range_for_export_function() {
        let code = "// Описание\nФункция ПубликацииERP() Экспорт\nКонецФункции";
        check_diagnostics_snapshot_for(
            code,
            DiagnosticCode::MissingReturnedValueDescription,
            expect![[r#"
                MissingReturnedValueDescription @ 2:9..2:22
                  message: Добавьте описание возвращаемого значения функции
                  severity: Warning"#]],
        );
        let diagnostics = check_ast_diagnostic(code, check);

        assert_eq!(diagnostics.len(), 1, "Should have one diagnostic");

        let range = diagnostics[0].range;
        let start: usize = range.start().into();
        let end: usize = range.end().into();
        let highlighted_text = &code[start..end];

        assert_eq!(
            highlighted_text, "ПубликацииERP",
            "Should highlight only function name, not parameters or modifiers"
        );
    }

    #[test]
    fn test_diagnostic_range_mixed_cyrillic_latin() {
        let code = "// Описание\nФункция ЗапросВERP(СервисПубликации, ПараметрыЗапроса, Сессия = Неопределено) Экспорт\nКонецФункции";
        check_diagnostics_snapshot_for(
            code,
            DiagnosticCode::MissingReturnedValueDescription,
            expect![[r#"
                MissingReturnedValueDescription @ 2:9..2:19
                  message: Добавьте описание возвращаемого значения функции
                  severity: Warning"#]],
        );
        let diagnostics = check_ast_diagnostic(code, check);

        assert_eq!(diagnostics.len(), 1, "Should have one diagnostic");

        let range = diagnostics[0].range;
        let start: usize = range.start().into();
        let end: usize = range.end().into();
        let highlighted_text = &code[start..end];

        assert_eq!(
            highlighted_text, "ЗапросВERP",
            "Should highlight only function name 'ЗапросВERP', got '{}'",
            highlighted_text
        );
    }

    #[test]
    fn test_non_export_function_no_diagnostic() {
        let code = "// Описание\nФункция НастройкиПодключения(СервисПубликации)\n\tВозврат Новый Структура;\nКонецФункции";
        check_diagnostics_snapshot_for(
            code,
            DiagnosticCode::MissingReturnedValueDescription,
            expect![[r#""#]],
        );
    }

    #[test]
    fn test_export_function_requires_documentation() {
        let code =
            "// Описание\nФункция НастройкиПодключения(СервисПубликации) Экспорт\n\tВозврат Новый Структура;\nКонецФункции";
        check_diagnostics_snapshot_for(
            code,
            DiagnosticCode::MissingReturnedValueDescription,
            expect![[r#"
                MissingReturnedValueDescription @ 2:9..2:29
                  message: Добавьте описание возвращаемого значения функции
                  severity: Warning"#]],
        );
    }

    #[test]
    fn test_export_function_with_complete_docs_ok() {
        let code = "// Описание\n// Возвращаемое значение:\n//  Структура - настройки подключения\nФункция НастройкиПодключения(СервисПубликации) Экспорт\n\tВозврат Новый Структура;\nКонецФункции";
        check_diagnostics_snapshot_for(
            code,
            DiagnosticCode::MissingReturnedValueDescription,
            expect![[r#""#]],
        );
    }

    #[test]
    fn test_export_function_with_collection_return_docs_ok() {
        let code = r#"// Возвращает хранимые файлы.
//
// Возвращаемое значение:
//   Массив из см. РаботаСФайлами.ДанныеФайла
//
Функция ПолучитьХранимыеФайлы(ВнешнийОбъект) Экспорт
    Возврат Новый Массив;
КонецФункции"#;

        check_diagnostics_snapshot_for(
            code,
            DiagnosticCode::MissingReturnedValueDescription,
            expect![[r#""#]],
        );
    }

    #[test]
    fn test_fields_without_main_type_triggers_diagnostic() {
        let code = r#"// Пакет ответа результата вызова метода HTTP.
//
// Возвращаемое значение:
//   * Метод - Строка - имя HTTP-метода запроса
//   * URL - Строка - итоговый URL, по которому был выполнен запрос.
//   * КодСостояния - Число - Код состояния ответа.
//   * Заголовки - Соответствие - Заголовки ответа.
//   * Тело - ДвоичныеДанные - Тело ответа.
//   * Кодировка - Строка - код кодировки ответа.
//
Функция НовыйОтвет() Экспорт
    Возврат Новый Структура;
КонецФункции"#;
        check_diagnostics_snapshot_for(
            code,
            DiagnosticCode::MissingReturnedValueDescription,
            expect![[r#"
                MissingReturnedValueDescription @ 11:9..11:19
                  message: Добавьте описание возвращаемого значения функции
                  severity: Warning"#]],
        );
    }

    #[test]
    fn test_see_link_in_parameter_description_does_not_excuse_missing_returns() {
        // bsl-ls stays silent here because of the `см.` link inside the
        // parameter description; the return value is still undocumented, so
        // we report it.
        let code = r#"// ++ 23.07.26 Мамонтов. Сделать простановку документа Транспортировка к выгрузке.
// Параметры:
//  Транспортировка - ДокументСсылка.Транспортировка - заполняемая транспортировка.
//  НеРегистрироватьВШине - Булево - Истина, если вызов выполняется внутри записи документа
//                          Транспортировка (см. ЗаполнитьПлечиДоставки). При вызове извне
//                          записи документа параметр указывать не нужно.
//
// -- 23.07.26 Мамонтов. Сделать простановку документа Транспортировка к выгрузке.
Функция ЗаполнитьПеревозчикаПоПлечамДоставки(Транспортировка, НеРегистрироватьВШине = Ложь) Экспорт
    Возврат Истина;
КонецФункции"#;
        check_diagnostics_snapshot_for(
            code,
            DiagnosticCode::MissingReturnedValueDescription,
            expect![[r#"
                MissingReturnedValueDescription @ 9:9..9:45
                  message: Добавьте описание возвращаемого значения функции
                  severity: Warning"#]],
        );
    }

    #[test]
    fn test_detached_comment_block_is_not_documentation() {
        // A comment separated from the function by a blank line is not its
        // documentation, so the returns-description check has nothing to say.
        let code = r#"// ++ SDG 26.12.2024 Добавить Исполнителя на вкладке В пути.

Функция ЗаполнитьПеревозчика(Транспортировка) Экспорт
    Возврат Истина;
КонецФункции"#;
        check_diagnostics_snapshot_for(
            code,
            DiagnosticCode::MissingReturnedValueDescription,
            expect![[r#""#]],
        );
    }

    #[test]
    fn test_fields_with_main_type_ok() {
        let code = r#"// Пакет ответа результата вызова метода HTTP.
//
// Возвращаемое значение:
//   Структура:
//   * Метод - Строка - имя HTTP-метода запроса
//   * URL - Строка - итоговый URL, по которому был выполнен запрос.
//   * КодСостояния - Число - Код состояния ответа.
//
Функция НовыйОтвет() Экспорт
    Возврат Новый Структура;
КонецФункции"#;
        check_diagnostics_snapshot_for(
            code,
            DiagnosticCode::MissingReturnedValueDescription,
            expect![[r#""#]],
        );
    }
}
