use bsl_metadata::metadata_object::MdoType;
use bsl_metadata::traits::MdObject;
use bsl_metadata::Configuration;
use rmcp::model::{CallToolResult, Content};
use rmcp::ErrorData as McpError;
use std::collections::BTreeMap;
use std::fmt::Write;

pub async fn get_live_metadata_tree(
    state: &crate::SharedState,
    connection: Option<&str>,
    meta_type: &str,
    name_mask: Option<String>,
    limit: u32,
) -> Result<CallToolResult, McpError> {
    let selected =
        state.onec_connection(connection).map_err(|e| McpError::invalid_params(e, None))?;
    let result = selected
        .client()
        .list_metadata(&onec_client::MetadataListRequest {
            meta_type: meta_type.to_string(),
            name_mask,
            limit: limit.clamp(1, 1000),
        })
        .await
        .map_err(|e| McpError::internal_error(format!("Ошибка чтения метаданных 1С: {e}"), None))?;
    Ok(crate::tools::response::structured(serde_json::json!({
        "source": "infobase",
        "connection": connection,
        "items": result.items.into_iter().map(|item| serde_json::json!({
            "name": item.name,
            "full_name": item.full_name,
            "synonym": item.synonym,
        })).collect::<Vec<_>>(),
        "returned": result.returned,
        "truncated": result.truncated,
    })))
}

pub async fn get_live_metadata_object(
    state: &crate::SharedState,
    connection: Option<&str>,
    meta_type: &str,
    name: &str,
) -> Result<CallToolResult, McpError> {
    let selected =
        state.onec_connection(connection).map_err(|e| McpError::invalid_params(e, None))?;
    let value = selected
        .client()
        .metadata_structure(&onec_client::MetadataStructureRequest {
            meta_type: meta_type.to_string(),
            name: name.to_string(),
        })
        .await
        .map_err(|e| McpError::internal_error(format!("Ошибка чтения метаданных 1С: {e}"), None))?;
    Ok(crate::tools::response::structured(serde_json::json!({
        "source": "infobase",
        "connection": connection,
        "object": value,
    })))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServiceKind {
    Http,
    Web,
    Integration,
}

fn parse_service_kind(raw: &str) -> Option<ServiceKind> {
    match raw.to_lowercase().as_str() {
        "httpservice" | "httpservices" | "httpсервис" | "httpсервисы" => {
            Some(ServiceKind::Http)
        }
        "webservice" | "webservices" | "webсервис" | "webсервисы" => {
            Some(ServiceKind::Web)
        }
        "integrationservice" | "integrationservices" | "сервисинтеграции" | "сервисыинтеграции" => {
            Some(ServiceKind::Integration)
        }
        _ => None,
    }
}

fn dash(s: &str) -> &str {
    if s.is_empty() {
        "—"
    } else {
        s
    }
}

pub fn get_metadata_tree(
    config: &Configuration,
    extensions: &[(String, Configuration)],
    filter: Option<String>,
    max_output_tokens: usize,
) -> Result<CallToolResult, McpError> {
    if let Some(ref category) = filter {
        format_filtered_tree(config, category, max_output_tokens)
    } else {
        let mut result = format_summary_tree(config)?;
        if !extensions.is_empty() {
            let text = result.content[0].raw.as_text().expect("text").text.clone();
            let mut out = text;
            for (name, ext_config) in extensions {
                out.push_str(&format_extension_summary(name, ext_config));
            }
            result = CallToolResult::success(vec![Content::text(out)]);
        }
        Ok(result)
    }
}

fn format_summary_tree(config: &Configuration) -> Result<CallToolResult, McpError> {
    let mut categories: BTreeMap<&str, usize> = BTreeMap::new();

    for obj in config.metadata_objects() {
        let key = obj.mdo_type.russian_name();
        *categories.entry(key).or_default() += 1;
    }

    for reg in config.registers() {
        let key = reg.mdo_type().russian_name();
        *categories.entry(key).or_default() += 1;
    }

    let common_modules = config.common_modules().len();
    if common_modules > 0 {
        categories.insert("ОбщийМодуль", common_modules);
    }

    let event_subs = config.event_subscriptions().len();
    if event_subs > 0 {
        categories.insert("ПодпискаНаСобытие", event_subs);
    }

    let defined_types = config.defined_types().len();
    if defined_types > 0 {
        categories.insert("ОпределяемыйТип", defined_types);
    }

    let scheduled_jobs = config.scheduled_jobs().len();
    if scheduled_jobs > 0 {
        categories.insert("РегламентноеЗадание", scheduled_jobs);
    }

    let roles = config.roles().len();
    if roles > 0 {
        categories.insert("Роль", roles);
    }

    let http_services = config.http_services().len();
    if http_services > 0 {
        categories.insert("HTTPСервис", http_services);
    }

    let web_services = config.web_services().len();
    if web_services > 0 {
        categories.insert("WebСервис", web_services);
    }

    let integration_services = config.integration_services().len();
    if integration_services > 0 {
        categories.insert("СервисИнтеграции", integration_services);
    }

    let total: usize = categories.values().sum();
    let mut out = format!("# {} — дерево метаданных\n\n", config.name());
    let _ = writeln!(out, "Всего объектов: {total}\n");
    let _ = writeln!(out, "| Категория | Количество |");
    let _ = writeln!(out, "|-----------|------------|");
    for (category, count) in &categories {
        let _ = writeln!(out, "| {category} | {count} |");
    }
    let _ = writeln!(out, "\nИспользуйте `filter` для получения списка объектов категории.");

    Ok(CallToolResult::success(vec![Content::text(out)]))
}

/// Map a user-supplied category filter to a canonical category the tree understands.
///
/// The strict [`MdoType`] parser only knows the singular RU/EN names (`Справочник`/`Catalog`),
/// but an agent naturally tries the on-disk directory / plural forms (`Catalogs`, `Справочники`)
/// or over-qualifies a category with an object (`Справочник.Пользователи`). Normalise those to
/// the singular so the filter just works instead of erroring on a reasonable input.
fn normalize_filter_category(raw: &str) -> String {
    // `Справочник.Пользователи` — a category over-qualified with an object name; keep the head.
    let head = raw.split('.').next().unwrap_or(raw).trim();
    // English plurals ARE the directory names (`Catalogs`, `Documents`); stripping a trailing
    // ASCII `s` recovers the singular the parser knows, with no per-type table to maintain.
    let depluralised = head.strip_suffix('s').filter(|s| !s.is_empty());
    if let Some(en) = depluralised {
        if en.parse::<MdoType>().is_ok() {
            return en.to_string();
        }
    }
    // Russian plurals the parser does not enumerate (the few categories an agent commonly asks
    // for by plural). Anything else passes through unchanged for the parser / special-case match.
    match head.to_lowercase().as_str() {
        "справочники" => "Справочник".to_string(),
        "документы" => "Документ".to_string(),
        "отчеты" | "отчёты" => "Отчет".to_string(),
        "обработки" => "Обработка".to_string(),
        "перечисления" => "Перечисление".to_string(),
        "задачи" => "Задача".to_string(),
        "константы" => "Константа".to_string(),
        "бизнеспроцессы" => "БизнесПроцесс".to_string(),
        "планыобмена" => "ПланОбмена".to_string(),
        _ => head.to_string(),
    }
}

fn format_filtered_tree(
    config: &Configuration,
    raw_category: &str,
    max_output_tokens: usize,
) -> Result<CallToolResult, McpError> {
    let category = normalize_filter_category(raw_category);
    let category = category.as_str();
    let mut out = String::new();
    let mut found = false;

    if let Ok(mdo_type) = category.parse::<MdoType>() {
        let objects: Vec<_> =
            config.metadata_objects().iter().filter(|o| o.mdo_type == mdo_type).collect();

        if !objects.is_empty() {
            found = true;
            let _ = writeln!(out, "# {} ({})\n", mdo_type.russian_name(), objects.len());
            for obj in &objects {
                if let Some(ref name_en) = obj.name_en {
                    let _ = writeln!(out, "- {} ({name_en})", obj.name);
                } else {
                    let _ = writeln!(out, "- {}", obj.name);
                }
            }
        }

        let registers: Vec<_> =
            config.registers().iter().filter(|r| r.mdo_type() == mdo_type).collect();

        if !registers.is_empty() {
            found = true;
            let _ = writeln!(out, "# {} ({})\n", mdo_type.russian_name(), registers.len());
            for reg in &registers {
                let _ = writeln!(out, "- {}", reg.name());
            }
        }
    }

    if !found {
        if let Some(kind) = parse_service_kind(category) {
            found = format_service_listing(config, kind, &mut out);
        }
    }

    if !found {
        match category.to_lowercase().as_str() {
            "общиемодули" | "общиймодуль" | "commonmodule" | "commonmodules" =>
            {
                let modules = config.common_modules();
                if !modules.is_empty() {
                    found = true;
                    let _ = writeln!(out, "# ОбщиеМодули ({})\n", modules.len());
                    for m in modules {
                        let flags = format_common_module_flags(m);
                        let _ = writeln!(out, "- {} {flags}", m.name());
                    }
                }
            }
            "подписканасобытие" | "eventsubscription" | "eventsubscriptions" => {
                let subs = config.event_subscriptions();
                if !subs.is_empty() {
                    found = true;
                    let _ = writeln!(out, "# ПодпискиНаСобытия ({})\n", subs.len());
                    for s in subs {
                        let _ = writeln!(out, "- {}", s.name());
                    }
                }
            }
            "определяемыйтип" | "definedtype" | "definedtypes" => {
                let types = config.defined_types();
                if !types.is_empty() {
                    found = true;
                    let _ = writeln!(out, "# ОпределяемыеТипы ({})\n", types.len());
                    for t in types {
                        let _ = writeln!(out, "- {}", t.name());
                    }
                }
            }
            "роль" | "role" | "roles" => {
                let roles = config.roles();
                if !roles.is_empty() {
                    found = true;
                    let _ = writeln!(out, "# Роли ({})\n", roles.len());
                    for r in roles {
                        let _ = writeln!(out, "- {}", r.name());
                    }
                }
            }
            _ => {}
        }
    }

    if !found {
        return Err(McpError::invalid_params(
            format!("Категория '{raw_category}' не найдена. Вызовите get_metadata_tree без фильтра для списка категорий."),
            None,
        ));
    }

    // A category listing on a large config (e.g. every Справочник in ERP) can be thousands of
    // lines; cap it to the output budget at a line boundary so the response stays bounded.
    crate::tools::response::truncate_text_to_budget(
        &mut out,
        max_output_tokens,
        "\n-- список усечён под max_output_tokens; повысьте бюджет или запросите объект действием `object` --\n",
    );

    Ok(CallToolResult::success(vec![Content::text(out)]))
}

fn format_common_module_flags(m: &bsl_metadata::CommonModule) -> String {
    let mut flags = Vec::new();
    if m.is_server() {
        flags.push("Сервер");
    }
    if m.is_client_managed_application() {
        flags.push("Клиент");
    }
    if m.is_global() {
        flags.push("Глобальный");
    }
    if m.is_privileged() {
        flags.push("Привилегированный");
    }
    if m.is_server_call() {
        flags.push("ВызовСервера");
    }
    if flags.is_empty() {
        String::new()
    } else {
        format!("[{}]", flags.join(", "))
    }
}

fn format_service_listing(config: &Configuration, kind: ServiceKind, out: &mut String) -> bool {
    match kind {
        ServiceKind::Http => {
            let services = config.http_services();
            if services.is_empty() {
                return false;
            }
            let _ = writeln!(out, "# HTTPСервисы ({})\n", services.len());
            for service in services {
                let _ = writeln!(out, "- {}", service.name());
            }
            true
        }
        ServiceKind::Web => {
            let services = config.web_services();
            if services.is_empty() {
                return false;
            }
            let _ = writeln!(out, "# WebСервисы ({})\n", services.len());
            for service in services {
                let _ = writeln!(out, "- {}", service.name());
            }
            true
        }
        ServiceKind::Integration => {
            let services = config.integration_services();
            if services.is_empty() {
                return false;
            }
            let _ = writeln!(out, "# СервисыИнтеграции ({})\n", services.len());
            for service in services {
                let _ = writeln!(out, "- {}", service.name());
            }
            true
        }
    }
}

/// The whole-`Configuration` object formatter. Production resolves objects from the
/// resident substrate via [`object_from_db`] (sharing the `format_*` renderers below);
/// this variant is retained as the fixture-driven contract test for that formatting, so
/// it is test-only.
#[cfg(test)]
pub fn get_object_structure(
    config: &Configuration,
    object_type: &str,
    object_name: &str,
) -> Result<CallToolResult, McpError> {
    if let Some(kind) = parse_service_kind(object_type) {
        return get_service_structure(config, kind, object_name);
    }

    let mdo_type: MdoType =
        object_type.parse().map_err(|e: String| McpError::invalid_params(e, None))?;

    // An event subscription is not a data-bearing object: it lives in its own catalog,
    // so it is looked up by name rather than via `find_metadata_object`/`find_register`.
    if mdo_type == MdoType::EventSubscription {
        return match config.find_event_subscription(object_name) {
            Some(sub) => Ok(CallToolResult::success(vec![Content::text(
                format_event_subscription_structure(sub),
            )])),
            None => Err(McpError::invalid_params(
                format!("ПодпискаНаСобытие.{} не найдена в конфигурации", object_name),
                None,
            )),
        };
    }

    if let Some(obj) = config.find_metadata_object(mdo_type, object_name) {
        return Ok(CallToolResult::success(vec![Content::text(format_metadata_object_structure(
            obj, mdo_type,
        ))]));
    }

    if let Some(reg) = config.find_register_by_type_and_name(mdo_type, object_name) {
        return Ok(CallToolResult::success(vec![Content::text(format_register_structure(reg))]));
    }

    Err(McpError::invalid_params(
        format!("Объект {}.{} не найден в конфигурации", mdo_type.russian_name(), object_name),
        None,
    ))
}

#[cfg(test)]
fn get_service_structure(
    config: &Configuration,
    kind: ServiceKind,
    object_name: &str,
) -> Result<CallToolResult, McpError> {
    match kind {
        ServiceKind::Http => match config.find_http_service(object_name) {
            Some(service) => Ok(CallToolResult::success(vec![Content::text(
                format_http_service_structure(service),
            )])),
            None => Err(McpError::invalid_params(
                format!("HTTPService.{object_name} не найден в конфигурации"),
                None,
            )),
        },
        ServiceKind::Web => match config.find_web_service(object_name) {
            Some(service) => Ok(CallToolResult::success(vec![Content::text(
                format_web_service_structure(service),
            )])),
            None => Err(McpError::invalid_params(
                format!("WebService.{object_name} не найден в конфигурации"),
                None,
            )),
        },
        ServiceKind::Integration => match config.find_integration_service(object_name) {
            Some(service) => Ok(CallToolResult::success(vec![Content::text(
                format_integration_service_structure(service),
            )])),
            None => Err(McpError::invalid_params(
                format!("IntegrationService.{object_name} не найден в конфигурации"),
                None,
            )),
        },
    }
}

fn format_event_subscription_structure(sub: &bsl_metadata::EventSubscription) -> String {
    let mut out = format!("# ПодпискаНаСобытие.{}\n\n", sub.name());
    writeln!(out, "- Источник: {}", dash(sub.source())).ok();
    writeln!(out, "- Событие: {}", dash(sub.event())).ok();
    writeln!(out, "- Обработчик: {}", dash(sub.handler_string())).ok();
    if let Some(handler) = sub.parse_handler() {
        writeln!(out, "  - Модуль: {}", handler.module_name).ok();
        writeln!(out, "  - Метод: {}", handler.method_name).ok();
    }
    out
}

fn format_http_service_structure(service: &bsl_metadata::HTTPService) -> String {
    let mut out = format!("# HTTPService.{}\n\n", service.name());
    let _ = writeln!(out, "- RootURL: {}", dash(service.root_url()));
    if let Some(uri) = service.uri() {
        let _ = writeln!(out, "- Модуль: {uri}");
    }

    let templates = service.url_templates();
    if !templates.is_empty() {
        let _ = writeln!(out, "\n## URLTemplates ({})\n", templates.len());
        for template in templates {
            let _ = writeln!(out, "### {}", template.name());
            let _ = writeln!(out, "- URLTemplate: {}", dash(template.template()));
            let methods = template.methods();
            if !methods.is_empty() {
                let _ = writeln!(out, "\n| Метод | HTTPMethod | Обработчик |");
                let _ = writeln!(out, "|-------|------------|------------|");
                for method in methods {
                    let _ = writeln!(
                        out,
                        "| {} | {} | {} |",
                        method.name(),
                        dash(method.http_method()),
                        dash(method.handler())
                    );
                }
            }
            out.push('\n');
        }
    }

    out
}

fn format_web_service_structure(service: &bsl_metadata::WebService) -> String {
    let mut out = format!("# WebService.{}\n\n", service.name());
    let _ = writeln!(out, "- Namespace: {}", dash(service.namespace()));
    if let Some(uri) = service.uri() {
        let _ = writeln!(out, "- Модуль: {uri}");
    }

    let operations = service.operations();
    if !operations.is_empty() {
        let _ = writeln!(out, "\n## Операции ({})\n", operations.len());
        let _ = writeln!(out, "| Операция | Обработчик | Параметры |");
        let _ = writeln!(out, "|----------|------------|-----------|");
        for operation in operations {
            let parameters = operation
                .parameters()
                .iter()
                .map(bsl_metadata::WebServiceParameter::name)
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(
                out,
                "| {} | {} | {} |",
                operation.name(),
                dash(operation.procedure_name()),
                dash(&parameters)
            );
        }
    }

    out
}

fn format_integration_service_structure(service: &bsl_metadata::IntegrationService) -> String {
    let mut out = format!("# IntegrationService.{}\n\n", service.name());
    let channels = service.channels();
    if !channels.is_empty() {
        let _ = writeln!(out, "## Каналы ({})\n", channels.len());
        let _ = writeln!(out, "| Канал | ReceiveMessageProcessing |");
        let _ = writeln!(out, "|-------|--------------------------|");
        for channel in channels {
            let _ = writeln!(
                out,
                "| {} | {} |",
                channel.name(),
                dash(channel.receive_message_processing())
            );
        }
    }

    out
}

fn format_metadata_object_structure(
    obj: &bsl_metadata::MetadataObject,
    mdo_type: MdoType,
) -> String {
    let mut out = format!("# {}.{}\n\n", mdo_type.russian_name(), obj.name);

    if let Some(ref name_en) = obj.name_en {
        let _ = writeln!(out, "Английское имя: {name_en}\n");
    }

    if !obj.attributes.is_empty() {
        let _ = writeln!(out, "## Реквизиты ({})\n", obj.attributes.len());
        let _ = writeln!(out, "| Имя | Тип |");
        let _ = writeln!(out, "|-----|-----|");
        for attr in &obj.attributes {
            let _ = writeln!(out, "| {} | {} |", attr.name, attr.attr_type);
        }
        out.push('\n');
    }

    if !obj.tabular_sections.is_empty() {
        let _ = writeln!(out, "## Табличные части ({})\n", obj.tabular_sections.len());
        for ts in &obj.tabular_sections {
            let _ = writeln!(out, "### {}\n", ts.name());
            if !ts.attributes().is_empty() {
                let _ = writeln!(out, "| Имя | Тип |");
                let _ = writeln!(out, "|-----|-----|");
                for attr in ts.attributes() {
                    let _ = writeln!(out, "| {} | {} |", attr.name(), attr.attr_type());
                }
                out.push('\n');
            }
        }
    }

    if !obj.enum_values.is_empty() {
        let _ = writeln!(out, "## Значения перечисления ({})\n", obj.enum_values.len());
        for val in &obj.enum_values {
            if let Some(ref name_en) = val.name_en {
                let _ = writeln!(out, "- {} ({name_en})", val.name);
            } else {
                let _ = writeln!(out, "- {}", val.name);
            }
        }
        out.push('\n');
    }

    if !obj.predefined_items.is_empty() {
        let _ = writeln!(out, "## Предопределённые элементы ({})\n", obj.predefined_items.len());
        for item in &obj.predefined_items {
            let _ = writeln!(out, "- {}", item.name);
        }
    }

    out
}

fn format_register_structure(reg: &bsl_metadata::Register) -> String {
    let mut out = format!("# {}.{}\n\n", reg.mdo_type().russian_name(), reg.name());

    if !reg.dimensions().is_empty() {
        let _ = writeln!(out, "## Измерения ({})\n", reg.dimensions().len());
        let _ = writeln!(out, "| Имя | Тип |");
        let _ = writeln!(out, "|-----|-----|");
        for dim in reg.dimensions() {
            let type_str = dim
                .attr_type()
                .map(|t| t.to_string())
                .unwrap_or_else(|| dim.type_str().to_string());
            let _ = writeln!(out, "| {} | {type_str} |", dim.name());
        }
        out.push('\n');
    }

    if !reg.resources().is_empty() {
        let _ = writeln!(out, "## Ресурсы ({})\n", reg.resources().len());
        let _ = writeln!(out, "| Имя | Тип |");
        let _ = writeln!(out, "|-----|-----|");
        for res in reg.resources() {
            let type_str = res
                .attr_type()
                .map(|t| t.to_string())
                .unwrap_or_else(|| res.type_str().to_string());
            let _ = writeln!(out, "| {} | {type_str} |", res.name());
        }
        out.push('\n');
    }

    if !reg.attributes().is_empty() {
        let _ = writeln!(out, "## Реквизиты ({})\n", reg.attributes().len());
        let _ = writeln!(out, "| Имя | Тип |");
        let _ = writeln!(out, "|-----|-----|");
        for attr in reg.attributes() {
            let type_str = attr
                .attr_type()
                .map(|t| t.to_string())
                .unwrap_or_else(|| attr.type_str().to_string());
            let _ = writeln!(out, "| {} | {type_str} |", attr.name());
        }
        out.push('\n');
    }

    if let Some(periodicity) = reg.periodicity() {
        let _ = writeln!(out, "Периодичность: {periodicity:?}");
    }
    if let Some(register_type) = reg.register_type() {
        let _ = writeln!(out, "Вид регистра: {register_type:?}");
    }

    out
}

pub fn get_configuration_info(
    config: &Configuration,
    extensions: &[(String, Configuration)],
) -> Result<CallToolResult, McpError> {
    let total_objects = config.metadata_objects().len()
        + config.registers().len()
        + config.common_modules().len()
        + config.event_subscriptions().len()
        + config.defined_types().len()
        + config.scheduled_jobs().len()
        + config.roles().len()
        + config.http_services().len()
        + config.web_services().len()
        + config.integration_services().len();

    let mut out = format!("# Конфигурация: {}\n\n", config.name());
    let _ = writeln!(out, "- UUID: {}", config.uuid());
    let _ = writeln!(out, "- Всего объектов метаданных: {total_objects}");
    let _ = writeln!(out, "- Общие модули: {}", config.common_modules().len());
    let _ = writeln!(out, "- Объекты метаданных: {}", config.metadata_objects().len());
    let _ = writeln!(out, "- Регистры: {}", config.registers().len());
    let _ = writeln!(out, "- Подписки на события: {}", config.event_subscriptions().len());
    let _ = writeln!(out, "- Определяемые типы: {}", config.defined_types().len());
    let _ = writeln!(out, "- Регламентные задания: {}", config.scheduled_jobs().len());
    let _ = writeln!(out, "- Роли: {}", config.roles().len());
    let _ = writeln!(out, "- HTTP-сервисы: {}", config.http_services().len());
    let _ = writeln!(out, "- Web-сервисы: {}", config.web_services().len());
    let _ = writeln!(out, "- Сервисы интеграции: {}", config.integration_services().len());

    if !extensions.is_empty() {
        let _ = writeln!(out, "\n## Расширения ({})\n", extensions.len());
        for (name, ext_config) in extensions {
            let ext_objects = ext_config.metadata_objects().len()
                + ext_config.registers().len()
                + ext_config.common_modules().len();
            let _ = writeln!(out, "- **{name}**: {ext_objects} объектов");
        }
    }

    Ok(CallToolResult::success(vec![Content::text(out)]))
}

/// Read forms from disk. `source_root` MUST be the configuration root (the
/// `Configuration.xml`-bearing directory, e.g. `src/cf`) — passing the repo root when the
/// configuration is nested under `src/cf` makes every form look missing.
///
/// `object_name` is required for an object's forms (`<TypeDir>/<object>/Forms/…`) but ignored
/// for `CommonForm`, which is a top-level form with no parent object (`CommonForms/<Form>/…`).
pub fn get_form_structure(
    source_root: Option<&std::path::Path>,
    object_type: &str,
    object_name: Option<&str>,
    form_name: Option<&str>,
) -> Result<CallToolResult, McpError> {
    let root = source_root.ok_or_else(|| {
        McpError::invalid_params("Configuration root не задан, формы недоступны", None)
    })?;

    // CommonForm has no parent object: its forms live directly under `CommonForms/<Form>`,
    // not `<TypeDir>/<object>/Forms/<Form>`, so it takes a distinct path and no `object_name`.
    // `to_lowercase` (not `eq_ignore_ascii_case`) so the Cyrillic alias folds case too.
    if matches!(object_type.to_lowercase().as_str(), "commonform" | "общаяформа") {
        return forms_in_container(&root.join("CommonForms"), form_name, "ОбщаяФорма");
    }

    let object_name = object_name.ok_or_else(|| {
        McpError::invalid_params(
            "'object_name' обязателен для форм объекта (кроме CommonForm)",
            None,
        )
    })?;
    let type_dir = mdo_type_to_dir(object_type).ok_or_else(|| {
        McpError::invalid_params(format!("Неизвестный тип объекта: {object_type}"), None)
    })?;

    let forms_dir = root.join(type_dir).join(object_name).join("Forms");
    forms_in_container(&forms_dir, form_name, &format!("{object_type}.{object_name}"))
}

/// Read a single form (when `form_name` is given) or list every form directory inside
/// `container` (`.../Forms` for an object, `CommonForms` for a common form). `title` labels
/// the listing header and the empty-set error.
fn forms_in_container(
    container: &std::path::Path,
    form_name: Option<&str>,
    title: &str,
) -> Result<CallToolResult, McpError> {
    if !container.exists() {
        return Err(McpError::invalid_params(
            format!("Каталог форм не найден: {}", container.display()),
            None,
        ));
    }

    if let Some(fname) = form_name {
        let form_xml_path = container.join(fname).join("Ext").join("Form.xml");
        if !form_xml_path.exists() {
            return Err(McpError::invalid_params(
                format!("Форма не найдена: {}", form_xml_path.display()),
                None,
            ));
        }
        let xml = std::fs::read_to_string(&form_xml_path)
            .map_err(|e| McpError::internal_error(format!("Ошибка чтения формы: {e}"), None))?;
        let form = bsl_metadata::xml_parser::parse_form_xml(&xml)
            .map_err(|e| McpError::internal_error(format!("Ошибка разбора формы: {e}"), None))?;
        Ok(CallToolResult::success(vec![Content::text(format_form(&form, Some(fname)))]))
    } else {
        let mut form_names = Vec::new();
        if let Ok(entries) = std::fs::read_dir(container) {
            for entry in entries.flatten() {
                if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                    if let Some(name) = entry.file_name().to_str() {
                        form_names.push(name.to_string());
                    }
                }
            }
        }
        form_names.sort();

        if form_names.is_empty() {
            return Err(McpError::invalid_params(format!("Формы не найдены для {title}"), None));
        }

        let mut out = format!("# Формы {title}\n\n");
        for name in &form_names {
            let _ = writeln!(out, "- {name}");
        }
        let _ =
            writeln!(out, "\nИспользуйте `form_name` для получения структуры конкретной формы.");
        Ok(CallToolResult::success(vec![Content::text(out)]))
    }
}

/// Render a parsed form. `requested_name` is the form name the caller asked for: `Ext/Form.xml`
/// holds the managed-form *content* (items, attributes, handlers) but not the form's own name or
/// UUID — those live in the parent object's metadata — so the parsed name is usually empty and
/// the UUID nil. Fall back to the requested name for the heading, and label a nil UUID as
/// unavailable instead of printing a misleading all-zero one.
fn format_form(form: &bsl_metadata::Form, requested_name: Option<&str>) -> String {
    let title = match (form.name(), requested_name) {
        (name, _) if !name.is_empty() => name,
        (_, Some(requested)) => requested,
        (_, None) => "<имя недоступно>",
    };
    let mut out = format!("# Форма: {title}\n\n");
    let _ = writeln!(out, "- Тип: {:?}", form.form_type());
    let uuid = form.uuid();
    if uuid.is_nil() {
        let _ = writeln!(out, "- UUID: недоступен (хранится в метаданных объекта, не в Form.xml)");
    } else {
        let _ = writeln!(out, "- UUID: {uuid}");
    }

    if !form.attributes.is_empty() {
        let _ = writeln!(out, "\n## Реквизиты формы ({})\n", form.attributes.len());
        for attr in &form.attributes {
            let main_marker = if attr.is_main { " (основной)" } else { "" };
            let _ = writeln!(out, "- {}{}: {}", attr.name, main_marker, attr.attr_type);
            for col in &attr.columns {
                let _ = writeln!(out, "    - {}: {}", col.name, col.attr_type);
            }
        }
    }

    let elements = form.elements();
    if !elements.is_empty() {
        let _ = writeln!(out, "\n## Элементы ({})\n", elements.len());
        let _ = writeln!(out, "| Имя | DataPath |");
        let _ = writeln!(out, "|-----|----------|");
        for el in elements {
            let dp = el.data_path.as_deref().unwrap_or("—");
            let _ = writeln!(out, "| {} | {dp} |", el.name);
        }
    }

    if !form.event_handlers.is_empty() {
        let _ = writeln!(out, "\n## Обработчики событий ({})\n", form.event_handlers.len());
        for h in &form.event_handlers {
            let _ = writeln!(out, "- {}", h.handler_name);
        }
    }

    if !form.command_handlers.is_empty() {
        let _ = writeln!(out, "\n## Обработчики команд ({})\n", form.command_handlers.len());
        for h in &form.command_handlers {
            let _ = writeln!(out, "- {h}");
        }
    }

    out
}

fn format_extension_summary(name: &str, config: &Configuration) -> String {
    use std::collections::BTreeMap;
    let mut out = format!("\n---\n\n# Расширение: {name}\n\n");
    let mut categories: BTreeMap<&str, usize> = BTreeMap::new();

    for obj in config.metadata_objects() {
        let key = obj.mdo_type.russian_name();
        *categories.entry(key).or_default() += 1;
    }
    for reg in config.registers() {
        let key = reg.mdo_type().russian_name();
        *categories.entry(key).or_default() += 1;
    }
    let common_modules = config.common_modules().len();
    if common_modules > 0 {
        categories.insert("ОбщийМодуль", common_modules);
    }

    let total: usize = categories.values().sum();
    let _ = writeln!(out, "Всего объектов: {total}\n");
    let _ = writeln!(out, "| Категория | Количество |");
    let _ = writeln!(out, "|-----------|------------|");
    for (category, count) in &categories {
        let _ = writeln!(out, "| {category} | {count} |");
    }
    out
}

/// Whether the `object` action can resolve `object_type`, mirroring [`object_from_db`]'s
/// dispatch order: a service kind (`parse_service_kind`) FIRST, then an [`MdoType`]. The
/// force-rescan miss-retry gate uses this so it cannot drift from what the resolver
/// actually accepts — `MdoType::from_str` has NO service variants, so gating on it alone
/// would never retry a just-added HTTP/Web/Integration service (they'd stay "not found"
/// until the next periodic drift poll, unlike catalogs/registers/subscriptions).
pub(crate) fn is_resolvable_object_type(object_type: &str) -> bool {
    parse_service_kind(object_type).is_some() || object_type.parse::<MdoType>().is_ok()
}

/// The `metadata object` action reading the resident host: resolve the object across the
/// base configuration and every extension (a point-lookup on the per-MDO substrate, never
/// a whole-config load) and format it. The db counterpart of [`get_object_structure`];
/// they share the `format_*` renderers. Extension-merged visibility, so an object defined
/// only in an extension is found (wider than the retired base-only read).
pub fn object_from_db(
    db: &ide::RootDatabaseImpl,
    object_type: &str,
    object_name: &str,
) -> Result<CallToolResult, McpError> {
    if let Some(kind) = parse_service_kind(object_type) {
        return service_from_db(db, kind, object_name);
    }

    let mdo_type: MdoType =
        object_type.parse().map_err(|e: String| McpError::invalid_params(e, None))?;

    if mdo_type == MdoType::EventSubscription {
        return match db.resolve_event_subscription_across_roots(object_name) {
            Some(sub) => Ok(CallToolResult::success(vec![Content::text(
                format_event_subscription_structure(&sub),
            )])),
            None => Err(McpError::invalid_params(
                format!("ПодпискаНаСобытие.{object_name} не найдена в конфигурации"),
                None,
            )),
        };
    }

    if let Some(obj) = db.resolve_metadata_object_across_roots(mdo_type, object_name) {
        return Ok(CallToolResult::success(vec![Content::text(format_metadata_object_structure(
            &obj, mdo_type,
        ))]));
    }

    if let Some(reg) = db.resolve_register_across_roots(mdo_type, object_name) {
        return Ok(CallToolResult::success(vec![Content::text(format_register_structure(&reg))]));
    }

    Err(McpError::invalid_params(
        format!("Объект {}.{} не найден в конфигурации", mdo_type.russian_name(), object_name),
        None,
    ))
}

fn service_from_db(
    db: &ide::RootDatabaseImpl,
    kind: ServiceKind,
    object_name: &str,
) -> Result<CallToolResult, McpError> {
    match kind {
        ServiceKind::Http => match db.resolve_http_service_across_roots(object_name) {
            Some(service) => Ok(CallToolResult::success(vec![Content::text(
                format_http_service_structure(&service),
            )])),
            None => Err(McpError::invalid_params(
                format!("HTTPService.{object_name} не найден в конфигурации"),
                None,
            )),
        },
        ServiceKind::Web => match db.resolve_web_service_across_roots(object_name) {
            Some(service) => Ok(CallToolResult::success(vec![Content::text(
                format_web_service_structure(&service),
            )])),
            None => Err(McpError::invalid_params(
                format!("WebService.{object_name} не найден в конфигурации"),
                None,
            )),
        },
        ServiceKind::Integration => {
            match db.resolve_integration_service_across_roots(object_name) {
                Some(service) => Ok(CallToolResult::success(vec![Content::text(
                    format_integration_service_structure(&service),
                )])),
                None => Err(McpError::invalid_params(
                    format!("IntegrationService.{object_name} не найден в конфигурации"),
                    None,
                )),
            }
        }
    }
}

/// The base configuration plus each extension's, sourced from the resident db's Channel-2
/// `load_configuration` query (Salsa-cached, invalidated by metadata drift). The `tree`
/// and `info` enumeration payload — which inherently needs whole-config counts — after
/// the `MetadataCache` retirement. `object` never calls this (it stays on the substrate).
pub fn configs_from_db(
    db: &ide::RootDatabaseImpl,
) -> (Configuration, Vec<(String, Configuration)>) {
    let paths = db.all_config_paths();
    let base = paths
        .iter()
        .find(|(label, _)| label.is_none())
        .map(|(_, p)| (*db.configuration_for_root(p)).clone())
        .unwrap_or_else(|| Configuration::new("Configuration"));
    let extensions = paths
        .iter()
        .filter_map(|(label, p)| {
            label.as_ref().map(|name| (name.clone(), (*db.configuration_for_root(p)).clone()))
        })
        .collect();
    (base, extensions)
}

/// A "still loading, retry shortly" envelope for a metadata call issued while the resident
/// db is building (or rebuilding after an idle eviction). Never a hard "not loaded" error —
/// the resident always eventually becomes ready.
pub fn loading(report: &crate::diagnostics_state::StatusReport) -> CallToolResult {
    let mut msg = String::from("Метаданные загружаются, повторите запрос через несколько секунд.");
    if let Some(ms) = report.elapsed_ms {
        let _ = write!(msg, " (идёт загрузка, {ms} мс)");
    }
    CallToolResult::success(vec![Content::text(msg)])
}

#[cfg(test)]
fn fixture_config() -> Configuration {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../bsl-metadata/fixtures/designer");
    bsl_metadata::load_from_directory(path).expect("failed to load fixture configuration")
}

fn mdo_type_to_dir(object_type: &str) -> Option<&'static str> {
    match object_type.to_lowercase().as_str() {
        "catalog" | "справочник" => Some("Catalogs"),
        "document" | "документ" => Some("Documents"),
        "dataprocessor" | "обработка" => Some("DataProcessors"),
        "report" | "отчет" | "отчёт" => Some("Reports"),
        "enum" | "перечисление" => Some("Enums"),
        "chartofcharacteristictypes" | "планвидовхарактеристик" => {
            Some("ChartsOfCharacteristicTypes")
        }
        "chartofaccounts" | "плансчетов" => Some("ChartsOfAccounts"),
        "exchangeplan" | "планобмена" => Some("ExchangePlans"),
        "businessprocess" | "бизнеспроцесс" => Some("BusinessProcesses"),
        "task" | "задача" => Some("Tasks"),
        "eventsubscription" | "подписканасобытие" => Some("EventSubscriptions"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract_text(result: &CallToolResult) -> &str {
        result.content[0].raw.as_text().expect("expected text content").text.as_str()
    }

    #[test]
    fn get_form_structure_lists_forms_under_the_given_root() {
        // The object form directory is `<root>/<TypeDir>/<object>/Forms`. Passing the repo root
        // when the configuration is nested under `src/cf` makes every form look missing — so the
        // caller must pass the configuration root. This locks that the function resolves forms
        // relative to whatever root it is handed (the config root in production).
        let tmp = tempfile::tempdir().unwrap();
        let forms = tmp.path().join("Catalogs").join("Пользователи").join("Forms");
        std::fs::create_dir_all(forms.join("ФормаСписка")).unwrap();
        std::fs::create_dir_all(forms.join("ФормаЭлемента")).unwrap();

        let result =
            get_form_structure(Some(tmp.path()), "Catalog", Some("Пользователи"), None).unwrap();
        let text = extract_text(&result);
        assert!(text.contains("ФормаСписка"), "should list ФормаСписка: {text}");
        assert!(text.contains("ФормаЭлемента"), "should list ФормаЭлемента: {text}");

        // A wrong root (the repo root, missing the `src/cf` config segment) finds nothing.
        let repo_root = tmp.path().join("repo_root_without_config");
        std::fs::create_dir_all(&repo_root).unwrap();
        assert!(
            get_form_structure(Some(&repo_root), "Catalog", Some("Пользователи"), None).is_err(),
            "a root without the object tree must error, not silently succeed",
        );
    }

    #[test]
    fn get_form_structure_lists_common_forms_without_object_name() {
        // CommonForm is a top-level form: it lives under `CommonForms/<Form>`, has no parent
        // object, and so must resolve with object_name = None (the protocol's failing case).
        let tmp = tempfile::tempdir().unwrap();
        let common = tmp.path().join("CommonForms");
        std::fs::create_dir_all(common.join("ОтправкаSMS")).unwrap();
        std::fs::create_dir_all(common.join("Настройки")).unwrap();

        let result = get_form_structure(Some(tmp.path()), "CommonForm", None, None).unwrap();
        let text = extract_text(&result);
        assert!(text.contains("ОтправкаSMS"), "should list ОтправкаSMS: {text}");
        assert!(text.contains("Настройки"), "should list Настройки: {text}");

        // The localized type name resolves to the same place.
        assert!(get_form_structure(Some(tmp.path()), "ОбщаяФорма", None, None).is_ok());
    }

    #[test]
    fn get_form_structure_requires_object_name_for_object_forms() {
        let tmp = tempfile::tempdir().unwrap();
        let err = get_form_structure(Some(tmp.path()), "Catalog", None, None).unwrap_err();
        assert!(
            format!("{err:?}").contains("object_name"),
            "object forms must still require object_name: {err:?}",
        );
    }

    #[test]
    fn test_metadata_tree_summary() {
        let config = fixture_config();
        let result = get_metadata_tree(&config, &[], None, usize::MAX).unwrap();
        let text = extract_text(&result);

        assert!(text.contains("дерево метаданных"), "should have header");
        assert!(text.contains("Всего объектов:"), "should have total");
        assert!(text.contains("Справочник"), "should list catalogs");
        assert!(text.contains("Документ"), "should list documents");
    }

    #[test]
    fn test_metadata_tree_filter_catalogs() {
        let config = fixture_config();
        let result =
            get_metadata_tree(&config, &[], Some("Справочник".into()), usize::MAX).unwrap();
        let text = extract_text(&result);

        assert!(text.contains("Справочник"), "should have category name");
        assert!(text.contains("Справочник1"), "should list Справочник1");
    }

    #[test]
    fn filter_category_accepts_plural_and_overqualified_aliases() {
        // English directory/plural form.
        assert_eq!(normalize_filter_category("Catalogs"), "Catalog");
        assert_eq!(normalize_filter_category("Documents"), "Document");
        // Russian plural.
        assert_eq!(normalize_filter_category("Справочники"), "Справочник");
        assert_eq!(normalize_filter_category("Отчёты"), "Отчет");
        // Over-qualified with an object name — keep the category head.
        assert_eq!(normalize_filter_category("Справочник.Пользователи"), "Справочник");
        // The canonical singular passes through untouched.
        assert_eq!(normalize_filter_category("Справочник"), "Справочник");
        // A non-plural unknown is left for the parser / special-case match to handle.
        assert_eq!(normalize_filter_category("ОбщиеМодули"), "ОбщиеМодули");
    }

    #[test]
    fn test_metadata_tree_filter_plural_and_dotted_resolve() {
        let config = fixture_config();
        for input in ["Catalogs", "Справочники", "Справочник.Справочник1"]
        {
            let result = get_metadata_tree(&config, &[], Some(input.into()), usize::MAX).unwrap();
            let text = extract_text(&result);
            assert!(
                text.contains("Справочник1"),
                "filter `{input}` should list Справочник1: {text}"
            );
        }
    }

    #[test]
    fn test_metadata_tree_filter_common_modules() {
        let config = fixture_config();
        let result =
            get_metadata_tree(&config, &[], Some("ОбщиеМодули".into()), usize::MAX).unwrap();
        let text = extract_text(&result);

        assert!(text.contains("ОбщиеМодули"), "should have category name");
    }

    #[test]
    fn test_metadata_tree_filter_invalid() {
        let config = fixture_config();
        let result =
            get_metadata_tree(&config, &[], Some("НесуществующаяКатегория".into()), usize::MAX);

        assert!(result.is_err(), "should return error for unknown category");
    }

    #[test]
    fn test_object_structure_catalog() {
        let config = fixture_config();
        let result = get_object_structure(&config, "Catalog", "Справочник1").unwrap();
        let text = extract_text(&result);

        assert!(text.contains("Справочник.Справочник1"), "should have object header");
    }

    #[test]
    fn test_object_structure_event_subscription() {
        // Regression: `ПодпискаНаСобытие` used to fail to parse as an MDO type ("Unknown
        // MDO type"); it now resolves and reports its source/event/handler.
        let config = fixture_config();
        let result =
            get_object_structure(&config, "ПодпискаНаСобытие", "ПриЗаписиСправочника").unwrap();
        let text = extract_text(&result);
        assert!(text.contains("ПодпискаНаСобытие.ПриЗаписиСправочника"), "header");
        assert!(text.contains("Событие: OnWrite"), "event");
        assert!(
            text.contains("Обработчик: CommonModule.ОбщийПодпискиНаСобытия.ПриЗаписиСправочника"),
            "handler string"
        );
        assert!(text.contains("Метод: ПриЗаписиСправочника"), "parsed handler method");
    }

    #[test]
    fn test_object_structure_event_subscription_not_found_is_graceful() {
        let config = fixture_config();
        // A missing subscription is an in-band invalid-params error, not a parse crash.
        assert!(get_object_structure(&config, "ПодпискаНаСобытие", "НетТакой").is_err());
    }

    #[test]
    fn test_object_structure_not_found() {
        let config = fixture_config();
        let result = get_object_structure(&config, "Catalog", "НесуществующийОбъект");

        assert!(result.is_err(), "should return error for missing object");
    }

    #[test]
    fn test_object_structure_invalid_type() {
        let config = fixture_config();
        let result = get_object_structure(&config, "InvalidType", "Test");

        assert!(result.is_err(), "should return error for invalid type");
    }

    #[test]
    fn test_configuration_info() {
        let config = fixture_config();
        let result = get_configuration_info(&config, &[]).unwrap();
        let text = extract_text(&result);

        assert!(text.contains("# Конфигурация:"), "should have config header");
        assert!(text.contains("UUID:"), "should have UUID");
        assert!(text.contains("Общие модули:"), "should have common modules count");
        assert!(text.contains("Регистры:"), "should have registers count");
    }

    #[test]
    fn test_form_structure_list_forms() {
        let fixture_root =
            concat!(env!("CARGO_MANIFEST_DIR"), "/../bsl-metadata/fixtures/designer");
        let root = std::path::Path::new(fixture_root);
        let result = get_form_structure(Some(root), "Document", Some("Документ1"), None).unwrap();
        let text = extract_text(&result);

        assert!(text.contains("Формы Document.Документ1"), "should have forms header");
        assert!(text.contains("ФормаДокумента"), "should list document form");
        assert!(text.contains("ФормаСписка"), "should list list form");
    }

    #[test]
    fn test_form_structure_specific_form() {
        let fixture_root =
            concat!(env!("CARGO_MANIFEST_DIR"), "/../bsl-metadata/fixtures/designer");
        let root = std::path::Path::new(fixture_root);
        let result =
            get_form_structure(Some(root), "Document", Some("Документ1"), Some("ФормаДокумента"))
                .unwrap();
        let text = extract_text(&result);

        assert!(text.contains("# Форма:"), "should have form header");
        // The heading carries a name (parsed, or the requested form name as fallback when
        // Ext/Form.xml omits it), never a blank title.
        assert!(!text.contains("# Форма: \n"), "heading must not be blank: {text}");
        // A nil UUID (the common case — Form.xml carries no UUID) is labelled, never printed as
        // a misleading all-zero UUID.
        assert!(
            !text.contains("00000000-0000-0000-0000-000000000000"),
            "nil UUID must be labelled, not printed as zeros: {text}",
        );
    }

    #[test]
    fn test_form_structure_no_workspace() {
        let result = get_form_structure(None, "Catalog", Some("Test"), None);
        assert!(result.is_err(), "should fail without workspace root");
    }

    #[test]
    fn test_form_structure_form_not_found() {
        let fixture_root =
            concat!(env!("CARGO_MANIFEST_DIR"), "/../bsl-metadata/fixtures/designer");
        let root = std::path::Path::new(fixture_root);
        let result = get_form_structure(
            Some(root),
            "Document",
            Some("Документ1"),
            Some("НесуществующаяФорма"),
        );

        assert!(result.is_err(), "should fail for missing form");
    }

    // ===== Wave 2d: explicit service enumerate/object/config-info =====
    //
    // These pin the contract that the MCP metadata tools enumerate HTTP/Web/Integration
    // services as explicit category listings and produce structured object dumps, instead
    // of forcing the agent through the hot typed config resolver. They RED-fail until
    // `format_filtered_tree` grows HTTP/Web/Integration-service branches and
    // `get_object_structure` recognises the service MDO types.

    #[test]
    fn metadata_tree_filter_lists_http_services_explicitly() {
        let config = fixture_config();
        let result =
            get_metadata_tree(&config, &[], Some("HTTPСервис".into()), usize::MAX).unwrap();
        let text = extract_text(&result);

        assert!(text.contains("HTTPСервис1"), "filter must enumerate http services: {text}");
        assert!(
            text.contains("HTTP") && text.contains("Сервис"),
            "filter must label the http-service category explicitly: {text}"
        );
    }

    #[test]
    fn metadata_tree_filter_lists_web_services_explicitly() {
        let config = fixture_config();
        let result = get_metadata_tree(&config, &[], Some("WebСервис".into()), usize::MAX).unwrap();
        let text = extract_text(&result);

        assert!(text.contains("WebСервис1"), "filter must enumerate web services: {text}");
    }

    #[test]
    fn metadata_tree_filter_lists_integration_services_explicitly() {
        // IntegrationService is deliberately absent from `MetadataReferenceKind` (no
        // `Метаданные.СервисыИнтеграции` plural), but the MCP enumeration surface must
        // still list integration services as a discrete category for agent discovery.
        let config = fixture_config();
        let result =
            get_metadata_tree(&config, &[], Some("СервисИнтеграции".into()), usize::MAX).unwrap();
        let text = extract_text(&result);

        assert!(
            text.contains("ОбменСообщениями"),
            "filter must enumerate integration services: {text}"
        );
    }

    #[test]
    fn object_structure_dumps_http_service_explicitly() {
        let config = fixture_config();
        let result = get_object_structure(&config, "HTTPService", "HTTPСервис1").unwrap();
        let text = extract_text(&result);

        assert!(
            text.contains("HTTPService.HTTPСервис1") || text.contains("HTTPСервис1"),
            "object structure must name the http service: {text}"
        );
        assert!(
            text.contains("URLTemplate") || text.contains("Метод") || text.contains("RootURL"),
            "object structure must dump http-service shape (templates/methods/root): {text}"
        );
    }

    #[test]
    fn object_structure_dumps_web_service_explicitly() {
        let config = fixture_config();
        let result = get_object_structure(&config, "WebService", "WebСервис1").unwrap();
        let text = extract_text(&result);

        assert!(
            text.contains("WebService.WebСервис1") || text.contains("WebСервис1"),
            "object structure must name the web service: {text}"
        );
        assert!(
            text.contains("Операция") || text.contains("Namespace"),
            "object structure must dump web-service shape (operations/namespace): {text}"
        );
    }

    #[test]
    fn object_structure_dumps_integration_service_explicitly() {
        let config = fixture_config();
        let result =
            get_object_structure(&config, "IntegrationService", "ОбменСообщениями").unwrap();
        let text = extract_text(&result);

        assert!(
            text.contains("IntegrationService.ОбменСообщениями")
                || text.contains("ОбменСообщениями"),
            "object structure must name the integration service: {text}"
        );
        assert!(
            text.contains("Канал") || text.contains("Channel"),
            "object structure must dump integration-service channels: {text}"
        );
    }

    #[test]
    fn configuration_info_reports_http_and_web_service_counts_explicitly() {
        let config = fixture_config();
        let result = get_configuration_info(&config, &[]).unwrap();
        let text = extract_text(&result);

        assert!(
            text.contains("HTTP-сервисы:") && !text.contains("HTTP-сервисы: 0"),
            "configuration info must report non-zero http-service count: {text}"
        );
        assert!(
            text.contains("Web-сервисы:") && !text.contains("Web-сервисы: 0"),
            "configuration info must report non-zero web-service count: {text}"
        );
    }

    /// The force-rescan retry gate predicate must accept every type `object_from_db` can
    /// dispatch — including HTTP/Web/Integration services, which are NOT `MdoType` variants
    /// (they route through `parse_service_kind`). Gating on `MdoType::from_str` alone (the
    /// bug) would classify a service miss as non-retryable, so a just-added service would
    /// never force a re-scan.
    #[test]
    fn is_resolvable_object_type_covers_services_not_in_mdotype() {
        for t in
            ["HTTPService", "WebService", "IntegrationService", "httpсервис", "сервисинтеграции"]
        {
            assert!(
                t.parse::<MdoType>().is_err(),
                "{t} is deliberately not an MdoType — this is what the old gate missed",
            );
            assert!(
                is_resolvable_object_type(t),
                "{t} must be resolvable (dispatched via parse_service_kind)",
            );
        }
        for t in ["Catalog", "InformationRegister", "ПодпискаНаСобытие", "Справочник"]
        {
            assert!(is_resolvable_object_type(t), "{t} (an MdoType) must stay resolvable");
        }
        assert!(
            !is_resolvable_object_type("НеизвестныйТип"),
            "a genuinely unknown type must not be classified resolvable (no wasted force-scan)",
        );
    }
}
