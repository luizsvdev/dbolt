use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use dbolt_database::{
    ConnectionForm, ConnectionProfile, ConnectionScope, Driver, DriverRegistry, FormField,
    FormFieldKind, FormValues, SecretStore,
};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::form::{field, v_form};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::searchable_list::SearchableListItem;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{ActiveTheme, IndexPath, WindowExt};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::ui::connection_tree::group_segments;
use crate::util::ResultExt;

/// Password values are kept apart from the profile so they go to the secret
/// store and never into `profile.values`.
#[derive(Clone)]
pub struct SavedConnection {
    pub profile: ConnectionProfile,
    /// Empty password fields are left out, which keeps the stored secret
    /// when editing.
    pub secrets: Vec<(String, String)>,
}

pub enum ConnectionDialogEvent {
    Saved(SavedConnection),
}

#[derive(Clone)]
struct DriverChoice {
    id: SharedString,
    name: SharedString,
}

impl SearchableListItem for DriverChoice {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.name.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }
}

struct FieldInput {
    field: FormField,
    input: Entity<InputState>,
}

enum TestState {
    Idle,
    Running,
    Succeeded,
    Failed(String),
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ValidationErrors {
    pub name: Option<String>,
    pub fields: HashMap<&'static str, String>,
}

impl ValidationErrors {
    pub fn is_empty(&self) -> bool {
        self.name.is_none() && self.fields.is_empty()
    }
}

pub struct ConnectionDialog {
    registry: Arc<DriverRegistry>,
    secret_store: Arc<dyn SecretStore>,
    existing: Option<ConnectionProfile>,
    driver_select: Entity<SelectState<Vec<DriverChoice>>>,
    driver_id: Option<SharedString>,
    name_input: Entity<InputState>,
    group: Option<String>,
    global: bool,
    fields: Vec<FieldInput>,
    errors: ValidationErrors,
    driver_error: Option<String>,
    test_state: TestState,
    test_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ConnectionDialogEvent> for ConnectionDialog {}

impl ConnectionDialog {
    pub fn new(
        registry: Arc<DriverRegistry>,
        secret_store: Arc<dyn SecretStore>,
        existing: Option<ConnectionProfile>,
        default_group: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let choices: Vec<DriverChoice> = registry
            .iter()
            .map(|driver| DriverChoice {
                id: driver.metadata().id.into(),
                name: driver
                    .metadata()
                    .display_name
                    .into(),
            })
            .collect();
        let driver_id: Option<SharedString> = match &existing {
            Some(profile) => Some(
                profile
                    .driver_id
                    .clone()
                    .into(),
            ),
            None => choices
                .first()
                .map(|choice| choice.id.clone()),
        };
        let selected_index = driver_id
            .as_ref()
            .and_then(|id| {
                choices
                    .iter()
                    .position(|choice| &choice.id == id)
            })
            .map(IndexPath::new);
        let driver_select = cx.new(|cx| SelectState::new(choices, selected_index, window, cx));

        let name = existing
            .as_ref()
            .map(|profile| profile.name.clone())
            .unwrap_or_default();
        let group = existing
            .as_ref()
            .and_then(|profile| profile.group.clone())
            .or(default_group);
        let name_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("My database")
                .default_value(name)
        });
        let global = existing
            .as_ref()
            .is_some_and(|profile| profile.scope == ConnectionScope::Global);

        let subscriptions = vec![cx.subscribe_in(
            &driver_select,
            window,
            |this, _, event: &SelectEvent<Vec<DriverChoice>>, window, cx| {
                let SelectEvent::Confirm(Some(driver_id)) = event else {
                    return;
                };
                if this.driver_id.as_ref() != Some(driver_id) {
                    this.driver_id = Some(driver_id.clone());
                    this.rebuild_fields(window, cx);
                }
            },
        )];

        let mut dialog = Self {
            registry,
            secret_store,
            existing,
            driver_select,
            driver_id,
            name_input,
            group,
            global,
            fields: Vec::new(),
            errors: ValidationErrors::default(),
            driver_error: None,
            test_state: TestState::Idle,
            test_task: None,
            _subscriptions: subscriptions,
        };
        dialog.rebuild_fields(window, cx);
        dialog
    }

    fn driver(&self) -> Option<Arc<dyn Driver>> {
        self.driver_id
            .as_ref()
            .and_then(|id| self.registry.get(id))
            .cloned()
    }

    fn rebuild_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(driver) = self.driver() else {
            self.fields.clear();
            self.driver_error = Some("Choose a driver".to_string());
            cx.notify();
            return;
        };
        self.driver_error = None;
        let existing = self
            .existing
            .as_ref()
            .filter(|profile| profile.driver_id == driver.metadata().id);

        self.fields = driver
            .connection_form()
            .iter()
            .map(|form_field| {
                let input = cx.new(|cx| {
                    let state = InputState::new(window, cx);
                    if form_field.kind == FormFieldKind::Password {
                        let placeholder = if existing.is_some() {
                            "Leave empty to keep the saved password"
                        } else {
                            ""
                        };
                        return state
                            .masked(true)
                            .placeholder(placeholder);
                    }
                    let value = existing
                        .and_then(|profile| {
                            profile
                                .values
                                .get(form_field.id)
                        })
                        .map(String::as_str)
                        .or(form_field.default)
                        .unwrap_or_default()
                        .to_string();
                    state.default_value(value)
                });
                FieldInput {
                    field: form_field.clone(),
                    input,
                }
            })
            .collect();
        self.errors = ValidationErrors::default();
        self.test_state = TestState::Idle;
        self.test_task = None;
        cx.notify();
    }

    fn collect_values(&self, cx: &App) -> FormValues {
        self.fields
            .iter()
            .map(|field_input| {
                let value = field_input
                    .input
                    .read(cx)
                    .value();
                let value = if field_input.field.kind == FormFieldKind::Password {
                    value.to_string()
                } else {
                    value.trim().to_string()
                };
                (
                    field_input
                        .field
                        .id
                        .to_string(),
                    value,
                )
            })
            .collect()
    }

    fn validated(&mut self, cx: &mut Context<Self>) -> Option<(Arc<dyn Driver>, FormValues)> {
        let Some(driver) = self.driver() else {
            self.driver_error = Some("Choose a driver".to_string());
            cx.notify();
            return None;
        };
        let values = self.collect_values(cx);
        let name = self
            .name_input
            .read(cx)
            .value();
        self.errors = validate(
            &name,
            driver.connection_form(),
            &values,
            self.existing.is_some(),
        );
        cx.notify();
        self.errors
            .is_empty()
            .then_some((driver, values))
    }

    fn test_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((driver, mut values)) = self.validated(cx) else {
            return;
        };
        let missing_secrets: Vec<&'static str> = driver
            .connection_form()
            .iter()
            .filter(|form_field| form_field.kind == FormFieldKind::Password)
            .filter(|form_field| {
                values
                    .get(form_field.id)
                    .is_none_or(|value| value.is_empty())
            })
            .map(|form_field| form_field.id)
            .collect();
        let existing_id = self
            .existing
            .as_ref()
            .map(|profile| profile.id);
        let secret_store = self.secret_store.clone();

        self.test_state = TestState::Running;
        cx.notify();
        self.test_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if let Some(connection_id) = existing_id {
                        for field_id in missing_secrets {
                            if let Some(secret) = secret_store
                                .get(connection_id, field_id)
                                .map_err(|error| error.to_string())?
                            {
                                values.insert(field_id.to_string(), secret);
                            }
                        }
                    }
                    let connection = driver
                        .connect(&values)
                        .map_err(|error| error.to_string())?;
                    connection
                        .ping()
                        .map_err(|error| error.to_string())
                })
                .await;
            this.update(cx, |this, cx| {
                this.test_state = match result {
                    Ok(()) => TestState::Succeeded,
                    Err(message) => TestState::Failed(message),
                };
                cx.notify();
            })
            .log_err();
        }));
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((driver, values)) = self.validated(cx) else {
            return;
        };
        let scope = if self.global {
            ConnectionScope::Global
        } else {
            ConnectionScope::Project
        };
        let saved = build_saved_connection(
            self.existing.as_ref(),
            driver.metadata().id,
            &self
                .name_input
                .read(cx)
                .value(),
            self.group.as_deref(),
            scope,
            driver.connection_form(),
            &values,
        );
        cx.emit(ConnectionDialogEvent::Saved(saved));
        window.close_dialog(cx);
    }

    fn render_error(message: &str, cx: &App) -> impl IntoElement {
        div()
            .text_xs()
            .text_color(cx.theme().danger)
            .child(message.to_string())
    }
}

impl Render for ConnectionDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        let driver_field = field()
            .label("Driver")
            .required(true)
            .child(Select::new(&self.driver_select))
            .when_some(self.driver_error.as_deref(), |this, message| {
                this.child(Self::render_error(message, cx))
            });
        let name_field = field()
            .label("Name")
            .required(true)
            .child(Input::new(&self.name_input))
            .when_some(self.errors.name.as_deref(), |this, message| {
                this.child(Self::render_error(message, cx))
            });
        let scope_field = field().child(
            Checkbox::new("connection-global")
                .label("Global (available in every project)")
                .checked(self.global)
                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                    this.global = *checked;
                    cx.notify();
                })),
        );
        let driver_fields = self
            .fields
            .iter()
            .map(|field_input| {
                field()
                    .label(field_input.field.label)
                    .required(field_input.field.required)
                    .child(
                        Input::new(&field_input.input)
                            .when(field_input.field.kind == FormFieldKind::Password, |input| {
                                input.mask_toggle()
                            }),
                    )
                    .when_some(
                        self.errors
                            .fields
                            .get(field_input.field.id),
                        |this, message| this.child(Self::render_error(message, cx)),
                    )
            })
            .collect::<Vec<_>>();

        let test_result = match &self.test_state {
            TestState::Idle => None,
            TestState::Running => Some(
                div()
                    .text_color(theme.muted_foreground)
                    .child("Testing connection…"),
            ),
            TestState::Succeeded => Some(
                div()
                    .text_color(theme.success)
                    .child("Connection succeeded"),
            ),
            TestState::Failed(message) => Some(
                div()
                    .text_color(theme.danger)
                    .child(message.clone()),
            ),
        };
        let testing = matches!(self.test_state, TestState::Running);

        v_flex()
            .gap_3()
            .child(
                v_form()
                    .child(driver_field)
                    .child(name_field)
                    .children(driver_fields)
                    .child(scope_field),
            )
            .when_some(test_result, |this, result| this.child(result.text_sm()))
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        Button::new("test-connection")
                            .label("Test Connection")
                            .outline()
                            .loading(testing)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.test_connection(window, cx)),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("cancel-connection")
                                    .label("Cancel")
                                    .on_click(|_, window, cx| window.close_dialog(cx)),
                            )
                            .child(
                                Button::new("save-connection")
                                    .label("Save")
                                    .primary()
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.save(window, cx)),
                                    ),
                            ),
                    ),
            )
    }
}

/// A required password may stay empty when editing, because the saved one is
/// kept.
pub fn validate(
    name: &str,
    form: &ConnectionForm,
    values: &FormValues,
    editing: bool,
) -> ValidationErrors {
    let mut errors = ValidationErrors::default();
    if name.trim().is_empty() {
        errors.name = Some("Name is required".to_string());
    }
    for form_field in form {
        let value = values
            .get(form_field.id)
            .map(String::as_str)
            .unwrap_or_default();
        let keeps_saved_secret = editing && form_field.kind == FormFieldKind::Password;
        if value.is_empty() {
            if form_field.required && !keeps_saved_secret {
                errors
                    .fields
                    .insert(form_field.id, format!("{} is required", form_field.label));
            }
            continue;
        }
        if form_field.kind == FormFieldKind::Number
            && !value
                .chars()
                .all(|character| character.is_ascii_digit())
        {
            errors.fields.insert(
                form_field.id,
                format!("{} must be a number", form_field.label),
            );
        }
    }
    errors
}

pub fn build_saved_connection(
    existing: Option<&ConnectionProfile>,
    driver_id: &str,
    name: &str,
    group: Option<&str>,
    scope: ConnectionScope,
    form: &ConnectionForm,
    values: &FormValues,
) -> SavedConnection {
    let mut profile = match existing {
        Some(profile) => profile.clone(),
        None => ConnectionProfile::new(name, driver_id, scope),
    };
    profile.name = name.trim().to_string();
    profile.driver_id = driver_id.to_string();
    profile.scope = scope;
    let segments = group_segments(group);
    profile.group = (!segments.is_empty()).then(|| segments.join("/"));

    let mut profile_values = BTreeMap::new();
    let mut secrets = Vec::new();
    for form_field in form {
        let Some(value) = values
            .get(form_field.id)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        if form_field.kind == FormFieldKind::Password {
            secrets.push((form_field.id.to_string(), value.clone()));
        } else {
            profile_values.insert(form_field.id.to_string(), value.clone());
        }
    }
    profile.values = profile_values;

    SavedConnection { profile, secrets }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the glob would bring in GPUI's `test` macro, which
    // shadows the standard `#[test]` attribute.
    use dbolt_database::{
        ConnectionProfile, ConnectionScope, FormField, FormFieldKind, FormValues,
    };

    use super::{build_saved_connection, validate};

    const FORM: [FormField; 3] = [
        FormField {
            id: "host",
            label: "Host",
            kind: FormFieldKind::Text,
            required: true,
            default: None,
        },
        FormField {
            id: "port",
            label: "Port",
            kind: FormFieldKind::Number,
            required: false,
            default: None,
        },
        FormField {
            id: "password",
            label: "Password",
            kind: FormFieldKind::Password,
            required: true,
            default: None,
        },
    ];

    fn values(pairs: &[(&str, &str)]) -> FormValues {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn validate_reports_missing_and_invalid_fields() {
        let errors = validate(" ", &FORM, &values(&[("port", "54x")]), false);

        assert!(errors.name.is_some());
        assert!(
            errors
                .fields
                .contains_key("host")
        );
        assert!(
            errors
                .fields
                .contains_key("port")
        );
        assert!(
            errors
                .fields
                .contains_key("password")
        );
    }

    #[test]
    fn validate_accepts_empty_password_when_editing() {
        let errors = validate(
            "local",
            &FORM,
            &values(&[("host", "localhost"), ("port", "5432")]),
            true,
        );

        assert!(errors.is_empty());
    }

    #[test]
    fn passwords_go_to_secrets_not_profile_values() {
        let saved = build_saved_connection(
            None,
            "test-driver",
            " local ",
            Some("/work//prod/"),
            ConnectionScope::Global,
            &FORM,
            &values(&[("host", "localhost"), ("port", ""), ("password", "hunter2")]),
        );

        assert_eq!(saved.profile.name, "local");
        assert_eq!(saved.profile.group.as_deref(), Some("work/prod"));
        assert_eq!(saved.profile.scope, ConnectionScope::Global);
        assert!(
            !saved
                .profile
                .values
                .contains_key("password")
        );
        assert!(
            !saved
                .profile
                .values
                .contains_key("port")
        );
        assert_eq!(
            saved
                .profile
                .values
                .get("host")
                .map(String::as_str),
            Some("localhost")
        );
        assert_eq!(
            saved.secrets,
            vec![("password".to_string(), "hunter2".to_string())]
        );
    }

    #[test]
    fn editing_keeps_id_folder_and_drops_stale_values() {
        let mut existing = ConnectionProfile::new("old", "test-driver", ConnectionScope::Project);
        existing
            .values
            .insert("password".into(), "leaked".into());
        existing.group = Some("old".into());

        let saved = build_saved_connection(
            Some(&existing),
            "test-driver",
            "new",
            existing.group.as_deref(),
            ConnectionScope::Project,
            &FORM,
            &values(&[("host", "db"), ("password", "")]),
        );

        assert_eq!(saved.profile.id, existing.id);
        assert_eq!(saved.profile.group.as_deref(), Some("old"));
        assert!(
            !saved
                .profile
                .values
                .contains_key("password")
        );
        assert!(saved.secrets.is_empty());
    }
}
