use wellfriendpdf_engine::{
    extract_xfa, sanitize_xfa_pdf, xfa_flatten_pdf, xfa_inventory, xfa_runtime_report,
    xfa_security_report, ContentEngine, XfaFlattenMode, XfaFlattenOptions, XfaLimits,
    XfaRuntimeOptions, XfaSanitizerMode, XfaSanitizerOptions, XfaScriptPolicy, XfaSupportStatus,
};

struct PdfFixtureBuilder {
    objects: Vec<Vec<u8>>,
}

impl PdfFixtureBuilder {
    fn new() -> Self {
        Self {
            objects: Vec::new(),
        }
    }

    fn add(&mut self, body: impl AsRef<[u8]>) -> usize {
        self.objects.push(body.as_ref().to_vec());
        self.objects.len()
    }

    fn add_stream(&mut self, stream: &[u8]) -> usize {
        let mut body = format!("<< /Length {} >>\nstream\n", stream.len()).into_bytes();
        body.extend_from_slice(stream);
        body.extend_from_slice(b"\nendstream");
        self.add(body)
    }

    fn build(&self) -> Vec<u8> {
        let mut pdf = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, body) in self.objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            pdf.extend_from_slice(body);
            pdf.extend_from_slice(b"\nendobj\n");
        }
        let xref = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n", self.objects.len() + 1).as_bytes());
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF",
                self.objects.len() + 1
            )
            .as_bytes(),
        );
        pdf
    }
}

fn add_page_shell(builder: &mut PdfFixtureBuilder) {
    assert_eq!(
        builder.add("<< /Type /Catalog /Pages 2 0 R /AcroForm 6 0 R >>"),
        1
    );
    assert_eq!(builder.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"), 2);
    assert_eq!(
        builder.add(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] \
             /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>"
        ),
        3
    );
    assert_eq!(builder.add_stream(b"q Q\n"), 4);
    assert_eq!(
        builder.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"),
        5
    );
}

fn array_xfa_pdf(packets: &[(&str, &[u8])]) -> Vec<u8> {
    let mut builder = PdfFixtureBuilder::new();
    add_page_shell(&mut builder);
    let pairs = packets
        .iter()
        .enumerate()
        .map(|(index, (name, _))| format!("({name}) {} 0 R", index + 7))
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(builder.add(format!("<< /Fields [] /XFA [{pairs}] >>")), 6);
    for (_, bytes) in packets {
        builder.add_stream(bytes);
    }
    builder.build()
}

fn single_stream_xfa_pdf(xdp: &[u8]) -> Vec<u8> {
    let mut builder = PdfFixtureBuilder::new();
    add_page_shell(&mut builder);
    assert_eq!(builder.add("<< /Fields [] /XFA 7 0 R >>"), 6);
    assert_eq!(builder.add_stream(xdp), 7);
    builder.build()
}

const STATIC_TEMPLATE: &str = r#"<template xmlns="http://www.xfa.org/schema/xfa-template/3.3/">
  <subform name="form1" layout="position">
    <field name="name" x="20pt" y="20pt" w="180pt" h="24pt" mandatory="error">
      <caption><value><text>Full name</text></value></caption>
      <assist><toolTip>Name from the datasets packet</toolTip></assist>
      <value><text>Default name</text></value>
      <bind ref="$record.person.name"/>
      <ui><textEdit/></ui>
      <font typeface="Helvetica" size="10pt"/>
      <border presence="visible"/>
    </field>
    <field name="amount" x="20pt" y="52pt" w="180pt" h="24pt">
      <caption><value><text>Amount</text></value></caption>
      <value><decimal>0</decimal></value>
      <bind ref="$record.person.amount"/>
      <ui><numericEdit/></ui>
    </field>
    <field name="total" x="20pt" y="84pt" w="180pt" h="24pt">
      <caption><value><text>Total</text></value></caption>
      <value><decimal>0</decimal></value>
      <ui><numericEdit/></ui>
      <calculate><script contentType="application/x-formcalc">amount + 2</script></calculate>
    </field>
    <field name="unsafe" x="20pt" y="116pt" w="180pt" h="24pt">
      <value><text>unchanged</text></value>
      <event activity="click"><script contentType="application/x-javascript">app.launchURL('https://blocked.invalid')</script></event>
    </field>
    <draw name="notice" x="20pt" y="150pt" w="180pt" h="18pt">
      <value><text>Static XFA notice</text></value>
    </draw>
  </subform>
</template>"#;

const DATASETS: &str = r#"<datasets xmlns="http://www.xfa.org/schema/xfa-data/1.0/">
  <data><person><name>Alice Example</name><amount>3</amount></person></data>
</datasets>"#;

fn static_fixture() -> Vec<u8> {
    array_xfa_pdf(&[
        ("template", STATIC_TEMPLATE.as_bytes()),
        ("datasets", DATASETS.as_bytes()),
        (
            "connectionSet",
            br#"<connectionSet xmlns="http://www.xfa.org/schema/xfa-connection-set/2.8/"><wsdlConnection name="blocked"/></connectionSet>"#,
        ),
    ])
}

#[test]
fn array_inventory_and_static_extraction_are_ordered_bound_and_provenanced() {
    let engine = ContentEngine::open_bytes(static_fixture()).unwrap();
    let inventory = xfa_inventory(&engine, &XfaLimits::default()).unwrap();
    assert_eq!(inventory.schema_version, "prompt16.xfa.v1");
    assert_eq!(inventory.source_form, "array");
    assert_eq!(
        inventory.packet_order,
        ["template", "datasets", "connectionSet"]
    );
    assert!(inventory.packets.iter().all(|packet| {
        packet.parse_status == "parsed"
            && packet.object_reference.is_some()
            && !packet.content_sha256.is_empty()
    }));
    assert!(inventory.classification.static_xfa);
    assert!(!inventory.xml_safety.external_entities_enabled);
    assert!(!inventory.xml_safety.network_access_enabled);

    let extraction = extract_xfa(&engine, &XfaLimits::default()).unwrap();
    assert!(extraction.template_parsed);
    assert!(extraction.datasets_parsed);
    assert_eq!(extraction.fields.len(), 4);
    let name = extraction
        .fields
        .iter()
        .find(|field| field.name == "name")
        .unwrap();
    assert_eq!(name.caption.as_deref(), Some("Full name"));
    assert_eq!(name.value.as_deref(), Some("Alice Example"));
    assert_eq!(name.binding.mode, "ref");
    assert_eq!(name.binding.matched_nodes, 1);
    assert!(name.required);
    assert_eq!(name.provenance.packet, "template");
    assert!(name.provenance.source_start.is_some());
    assert!(extraction
        .semantic_integration
        .search_index_terms
        .iter()
        .any(|term| term == "alice"));
    assert!(!extraction.semantic_integration.rag_chunks.is_empty());
    assert_eq!(extraction.scripts.len(), 2);
    assert!(extraction
        .scripts
        .iter()
        .any(|script| script.language == "formcalc" && script.event == "calculate"));
    assert!(extraction
        .scripts
        .iter()
        .any(|script| script.language == "javascript" && script.event == "click"));
}

#[test]
fn single_stream_xdp_preserves_inherited_namespace_and_child_order() {
    let xdp = br#"<xdp:xdp xmlns:xdp="http://ns.adobe.com/xdp/" xmlns:xfa="http://www.xfa.org/schema/xfa-template/3.3/" xmlns:xfaData="http://www.xfa.org/schema/xfa-data/1.0/">
      <xfa:template><xfa:subform name="single"><xfa:field name="singleField"><xfa:value><xfa:text>single</xfa:text></xfa:value></xfa:field></xfa:subform></xfa:template>
      <xfaData:datasets><xfaData:data><singleField>bound</singleField></xfaData:data></xfaData:datasets>
    </xdp:xdp>"#;
    let engine = ContentEngine::open_bytes(single_stream_xfa_pdf(xdp)).unwrap();
    let inventory = xfa_inventory(&engine, &XfaLimits::default()).unwrap();
    assert_eq!(inventory.source_form, "single_stream");
    assert_eq!(inventory.packet_order, ["template", "datasets"]);
    assert_eq!(
        inventory.packets[0].xml_root_namespace.as_deref(),
        Some("http://www.xfa.org/schema/xfa-template/3.3/")
    );
    let extraction = extract_xfa(&engine, &XfaLimits::default()).unwrap();
    assert_eq!(extraction.fields.len(), 1);
    assert_eq!(extraction.fields[0].value.as_deref(), Some("bound"));
}

#[test]
fn duplicate_and_malformed_packets_are_exactly_reported_without_panicking() {
    let malformed = b"<!DOCTYPE template [<!ENTITY bomb SYSTEM 'file:///etc/passwd'>]><template>&bomb;</template>";
    let pdf = array_xfa_pdf(&[
        ("template", malformed),
        ("config", b"<config/>"),
        ("config", b"<config/>"),
    ]);
    let engine = ContentEngine::open_bytes(pdf).unwrap();
    let report = xfa_inventory(&engine, &XfaLimits::default()).unwrap();
    assert_eq!(report.status, XfaSupportStatus::UnsupportedReportedExact);
    assert!(report.packets[0].malformed);
    assert!(report.packets[2].duplicate);
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "xfa.xml.rejected"
            && diagnostic
                .message
                .contains("DTD/entity declarations are forbidden")
    }));
    assert!(report
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "xfa.packet.duplicate"));
}

#[test]
fn scripts_default_disabled_and_safe_formcalc_is_bounded() {
    let engine = ContentEngine::open_bytes(static_fixture()).unwrap();
    let disabled = xfa_runtime_report(&engine, &XfaRuntimeOptions::default()).unwrap();
    assert_eq!(disabled.sandbox.scripts_executed, 0);
    assert_eq!(disabled.sandbox.events_executed, 0);
    assert_eq!(disabled.sandbox.scripts_blocked, 2);
    assert!(!disabled.sandbox.network_access);
    assert!(!disabled.sandbox.filesystem_access);
    assert!(disabled.sandbox.no_secret_logging);

    let enabled = xfa_runtime_report(
        &engine,
        &XfaRuntimeOptions {
            script_policy: XfaScriptPolicy::FormCalcSafeSubset,
            execute_supported_events: true,
            ..XfaRuntimeOptions::default()
        },
    )
    .unwrap();
    assert_eq!(enabled.sandbox.scripts_executed, 1);
    assert_eq!(enabled.sandbox.events_executed, 1);
    assert_eq!(enabled.sandbox.scripts_blocked, 1);
    assert_eq!(enabled.sandbox.field_mutations, 1);
    assert!(enabled.sandbox.total_instructions > 0);
    assert!(
        enabled
            .layout_items
            .iter()
            .any(|item| { item.som_path.contains("total") && item.text.as_deref() == Some("5") }),
        "layout items: {:?}",
        enabled.layout_items
    );
    assert!(enabled
        .sandbox
        .audit_log
        .iter()
        .any(|entry| entry.reason_code == "xfa.script.javascript_or_proprietary_blocked"));
}

#[test]
fn dynamic_instances_overflow_deterministically_and_limits_fail_closed() {
    let template = r#"<template xmlns="http://www.xfa.org/schema/xfa-template/3.3/">
      <pageSet><pageArea name="p" w="200pt" h="120pt"><contentArea name="c" x="10pt" y="10pt" w="180pt" h="80pt"/></pageArea></pageSet>
      <subform name="root" layout="tb">
        <subform name="line" layout="tb" h="36pt"><occur min="1" max="4" initial="1"/><bind ref="$record.items.item"/>
          <field name="label" w="160pt" h="30pt"><value><text>row</text></value></field>
        </subform>
      </subform>
    </template>"#;
    let data = r#"<datasets xmlns="http://www.xfa.org/schema/xfa-data/1.0/"><data><items><item><label>A</label></item><item><label>B</label></item><item><label>C</label></item></items></data></datasets>"#;
    let engine = ContentEngine::open_bytes(array_xfa_pdf(&[
        ("template", template.as_bytes()),
        ("datasets", data.as_bytes()),
    ]))
    .unwrap();
    let first = xfa_runtime_report(&engine, &XfaRuntimeOptions::default()).unwrap();
    let second = xfa_runtime_report(&engine, &XfaRuntimeOptions::default()).unwrap();
    assert!(first.classification.dynamic_xfa);
    assert!(first.generated_instances >= 4);
    assert!(first.generated_pages >= 2);
    assert_eq!(first.layout_items, second.layout_items);
    let repeated_values = first
        .layout_items
        .iter()
        .filter(|item| item.kind == "field_value" && item.som_path.contains("line["))
        .map(|item| {
            (
                item.repeated_instance_index,
                item.text.as_deref(),
                item.som_path.as_str(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        repeated_values,
        vec![
            (0, Some("A"), "root[0].line[0].label[0]"),
            (1, Some("B"), "root[0].line[1].label[0]"),
            (2, Some("C"), "root[0].line[2].label[0]"),
        ]
    );

    let limits = XfaLimits {
        max_instances_per_subform: 2,
        ..XfaLimits::default()
    };
    let error = xfa_runtime_report(
        &engine,
        &XfaRuntimeOptions {
            limits,
            ..XfaRuntimeOptions::default()
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("instance count"));

    let limits = XfaLimits {
        max_generated_pages: 1,
        ..XfaLimits::default()
    };
    let error = xfa_runtime_report(
        &engine,
        &XfaRuntimeOptions {
            limits,
            ..XfaRuntimeOptions::default()
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("generated page count"));
}

#[test]
fn static_flatten_reopens_deterministically_and_remove_mode_drops_xfa() {
    let source = static_fixture();
    let options = XfaFlattenOptions {
        mode: XfaFlattenMode::FlattenSupportedStatic,
        ..XfaFlattenOptions::default()
    };
    let (first, report) = xfa_flatten_pdf(&source, &options).unwrap();
    let (second, _) = xfa_flatten_pdf(&source, &options).unwrap();
    assert_eq!(first, second);
    assert!(report.reopen_verification.reopened);
    assert!(report.reopen_verification.xfa_present_after);
    assert!(report.layout_items_written > 0);
    assert!(report.unrelated_page_content_preserved);
    assert!(!report.xfa_removed);

    let (removed, report) = xfa_flatten_pdf(
        &source,
        &XfaFlattenOptions {
            mode: XfaFlattenMode::FlattenAndRemoveXfa,
            ..XfaFlattenOptions::default()
        },
    )
    .unwrap();
    assert!(report.xfa_removed);
    assert!(!report.reopen_verification.xfa_present_after);
    let reopened = ContentEngine::open_bytes(removed).unwrap();
    assert_eq!(reopened.page_count().unwrap(), 1);
    assert!(reopened.render_page_png_fast(1, 72).is_ok());
}

#[test]
fn sanitizer_rescan_security_and_redaction_posture_are_explicit() {
    let source = static_fixture();
    let engine = ContentEngine::open_bytes(source.clone()).unwrap();
    let security = xfa_security_report(&engine, &XfaLimits::default()).unwrap();
    assert_eq!(security.script_count, 2);
    assert_eq!(security.external_connection_count, 1);
    assert!(
        !security
            .redaction_posture
            .secure_redaction_proven_without_flattening
    );
    assert!(security.redaction_posture.supported_text_visible_to_planner);

    let (neutralized, report) = sanitize_xfa_pdf(
        &source,
        &XfaSanitizerOptions {
            mode: XfaSanitizerMode::RemoveScriptsEventsConnections,
            ..XfaSanitizerOptions::default()
        },
    )
    .unwrap();
    assert!(report.post_sanitize_rescan_passed);
    assert_eq!(report.output_scripts, 0);
    assert_eq!(report.output_events, 0);
    assert_eq!(report.output_external_connections, 0);
    assert!(!report.xfa_removed);
    let after = extract_xfa(
        &ContentEngine::open_bytes(neutralized).unwrap(),
        &XfaLimits::default(),
    )
    .unwrap();
    assert_eq!(after.fields.len(), 4);

    let (removed, report) = sanitize_xfa_pdf(
        &source,
        &XfaSanitizerOptions {
            mode: XfaSanitizerMode::RemoveAllXfa,
            ..XfaSanitizerOptions::default()
        },
    )
    .unwrap();
    assert!(report.post_sanitize_rescan_passed);
    assert!(report.xfa_removed);
    assert!(
        !xfa_inventory(
            &ContentEngine::open_bytes(removed).unwrap(),
            &XfaLimits::default()
        )
        .unwrap()
        .present
    );
}

#[test]
fn deep_xml_and_javascript_host_access_fail_closed_with_exact_status() {
    let deep = format!(
        "<template>{}<field name=\"x\"/>{}</template>",
        "<subform>".repeat(12),
        "</subform>".repeat(12)
    );
    let engine =
        ContentEngine::open_bytes(array_xfa_pdf(&[("template", deep.as_bytes())])).unwrap();
    let limits = XfaLimits {
        max_xml_depth: 5,
        ..XfaLimits::default()
    };
    let inventory = xfa_inventory(&engine, &limits).unwrap();
    assert_eq!(inventory.status, XfaSupportStatus::UnsupportedReportedExact);
    assert!(inventory
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message.contains("depth")));

    let security = xfa_security_report(
        &ContentEngine::open_bytes(static_fixture()).unwrap(),
        &XfaLimits::default(),
    )
    .unwrap();
    assert_eq!(
        security.runtime_support_status,
        XfaSupportStatus::ImplementedWithLimits
    );
    assert!(security
        .sandbox_default_policy
        .contains("no_external_side_effects"));
}
