use metrics::{counter, gauge, histogram, Label};
use std::collections::BTreeMap;
use crate::compiler::prelude::*;

fn emit_metric(
    metric_name: Value,
    metric_value: Value,
    metric_type: Bytes,
    metric_labels: BTreeMap<KeyString, Value>,
) -> Resolved {
    let key = metric_name.try_bytes_utf8_lossy().unwrap().to_string();
    let labels: Vec<Label> = metric_labels
        .into_iter()
        .filter_map(|(key, value)| {
            if value.is_bytes() {
                Some(Label::new(
                    String::from(key.as_str()),
                    value.try_bytes_utf8_lossy().unwrap().to_string(),
                ))
            } else {
                None
            }
        })
        .collect();

    match metric_type.as_ref() {
        b"counter" => {
            let c = counter!(key, labels);
            c.increment(metric_value.try_integer()? as u64);
        },
        b"gauge" => {
            let g = gauge!(key, labels);
            g.set(metric_value.try_into_f64()?);
        },
        b"histogram" => {
            let h = histogram!(key, labels);
            h.record(metric_value.try_into_f64()?);
        },
        _ => todo!(),
    }

    Ok(Value::Null)
}

#[derive(Clone, Copy, Debug)]
pub struct EmitMetric;
impl Function for EmitMetric {
    fn identifier(&self) -> &'static str {
        "emit_metric"
    }

    fn examples(&self) -> &'static [Example] {
        &[Example {
            title: "emit a metric from VRL",
            source: r#"emit_metric!(s'success.count', 1, s'counter')"#,
            result: Ok(r#"No Result"#),
        }]
    }

    fn compile(
        &self,
        state: &state::TypeState,
        _ctx: &mut FunctionCompileContext,
        arguments: ArgumentList,
    ) -> Compiled {
        let metric_name = arguments.required("key");
        let metric_value = arguments.required("value");
        let metric_types = vec!["counter".into(), "gauge".into(), "histogram".into()];

        let metric_type = arguments
            .optional_enum("mtype", &metric_types, state)?
            .unwrap_or_else(|| "counter".into())
            .try_bytes()
            .expect("type not bytes");

        let metric_labels = arguments.optional("labels");

        Ok(EmitMetricFn {
            metric_name,
            metric_value,
            metric_type,
            metric_labels,
        }
        .as_expr())
    }

    fn parameters(&self) -> &'static [Parameter] {
        &[
            Parameter {
                keyword: "key",
                kind: kind::BYTES,
                required: true,
            },
            Parameter {
                keyword: "value",
                kind: kind::INTEGER | kind::FLOAT,
                required: true,
            },
            Parameter {
                keyword: "mtype",
                kind: kind::BYTES,
                required: false,
            },
            Parameter {
                keyword: "labels",
                kind: kind::OBJECT,
                required: false,
            },
        ]
    }
}

#[derive(Debug, Clone)]
struct EmitMetricFn {
    metric_name: Box<dyn Expression>,
    metric_value: Box<dyn Expression>,
    metric_type: Bytes,
    metric_labels: Option<Box<dyn Expression>>,
}

impl FunctionExpression for EmitMetricFn {
    fn resolve(&self, ctx: &mut Context) -> Resolved {
        let metric_name = self.metric_name.resolve(ctx)?;
        if !metric_name.is_bytes() {
            return Err(ExpressionError::from(ValueError::Expected {
                got: metric_name.kind(),
                expected: Kind::bytes(),
            }));
        }

        let metric_value = self.metric_value.resolve(ctx)?;
        if !(metric_value.is_integer() || metric_value.is_float()) {
            return Err(ExpressionError::from(ValueError::Expected {
                got: metric_name.kind(),
                expected: Kind::integer() | Kind::float(),
            }));
        }

        let metric_type = self.metric_type.clone();

        let metric_labels = match self.metric_labels.as_ref() {
            Some(v) => v.resolve(ctx)?.try_object()?,
            None => BTreeMap::new(),
        };
        emit_metric(metric_name, metric_value, metric_type, metric_labels)
    }

    fn type_def(&self, _: &state::TypeState) -> TypeDef {
        TypeDef::null().infallible()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::btreemap;
    use crate::value;
    use metrics::Key;
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};
    use metrics_util::{CompositeKey, MetricKind};
    use ordered_float::OrderedFloat;

    test_function![
        emit_metric  => EmitMetric;

        BadKey {
            args: func_args![key:
                btreemap! {
                    "lvl" => "info",
                },
                value: value!(1)
            ],
            want: Err(format!(r"expected string, got {{ lvl: string }}")),
            tdef: TypeDef::null().infallible(),
        }

        BadValue {
            args: func_args![
                key: b"some.key",
                value: btreemap! {
                    "lvl" => "info",
                },
            ],
            want: Err(format!(r"expected integer or float, got string")),
            tdef: TypeDef::null().infallible(),
        }

        BadLabels {
            args: func_args![
                key: b"some.key",
                value: 1,
                labels: b"foo",
            ],
            want: Err(format!(r"expected object, got string")),
            tdef: TypeDef::null().infallible(),
        }
    ];

    #[test]
    fn test_emit_metrics() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        recorder.install().expect("Should not fail");
        static COUNTER_METRIC_NAME: &'static str = "test_counter";
        static GAUGE_METRIC_NAME: &'static str = "test_gauge";
        static HISTOGRAM_METRIC_NAME: &'static str = "test_histo";

        let labels: BTreeMap<KeyString, Value> = btreemap! {
            KeyString::from("l1") => "v1",
            KeyString::from("non_string1") => 3,
            KeyString::from("non_string2") => vec![1,2,3],
            KeyString::from("l2") => "v2"
        };

        let mut emit_result = emit_metric(
            Value::from("test_counter"),
            Value::from(21),
            "counter".into(),
            labels.clone(),
        );
        assert!(emit_result.is_ok());

        emit_result = emit_metric(
            Value::from("test_counter"),
            Value::from(21),
            "counter".into(),
            labels.clone(),
        );
        assert!(emit_result.is_ok());

        emit_result = emit_metric(
            Value::from("test_gauge"),
            Value::from(42),
            "gauge".into(),
            labels.clone(),
        );
        assert!(emit_result.is_ok());

        emit_result = emit_metric(
            Value::from("test_histo"),
            Value::from(42),
            "histogram".into(),
            labels.clone(),
        );
        assert!(emit_result.is_ok());

        let result = snapshotter.snapshot().into_vec();
        assert!(!result.is_empty());

        assert_eq!(
            result,
            vec![
                (
                    CompositeKey::new(
                        MetricKind::Counter,
                        Key::from_parts(
                            &COUNTER_METRIC_NAME[..],
                            vec![Label::new("l1", "v1"), Label::new("l2", "v2")]
                        )
                    ),
                    None,
                    None,
                    DebugValue::Counter(42),
                ),
                (
                    CompositeKey::new(
                        MetricKind::Gauge,
                        Key::from_parts(
                            &GAUGE_METRIC_NAME[..],
                            vec![Label::new("l1", "v1"), Label::new("l2", "v2")]
                        )
                    ),
                    None,
                    None,
                    DebugValue::Gauge(OrderedFloat::from(42.0)),
                ),
                (
                    CompositeKey::new(
                        MetricKind::Histogram,
                        Key::from_parts(
                            &HISTOGRAM_METRIC_NAME[..],
                            vec![Label::new("l1", "v1"), Label::new("l2", "v2")]
                        )
                    ),
                    None,
                    None,
                    DebugValue::Histogram(vec![OrderedFloat::from(42.0)]),
                )
            ]
        );
    }
}
