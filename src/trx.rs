//! Reads Visual Studio TRX result files.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};

use crate::report::{Counts, Failure};

pub struct TrxResults {
    pub counts: Counts,
    pub failures: Vec<Failure>,
}

pub fn read(path: &Path) -> Result<TrxResults> {
    let raw = std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    parse(raw.trim_start_matches('\u{feff}'))
}

pub fn parse(text: &str) -> Result<TrxResults> {
    let doc = roxmltree::Document::parse(text).context("invalid TRX file")?;

    // Some adapters put only the method name in testName. The class comes
    // from the test definition.
    let mut classes = HashMap::new();
    for def in doc.descendants().filter(|n| n.has_tag_name("UnitTest")) {
        let class = def
            .children()
            .find(|n| n.has_tag_name("TestMethod"))
            .and_then(|m| m.attribute("className"));
        if let (Some(id), Some(class)) = (def.attribute("id"), class) {
            classes.insert(id, class);
        }
    }

    let mut counts = Counts::default();
    let mut failures = Vec::new();
    for result in doc.descendants().filter(|n| n.has_tag_name("UnitTestResult")) {
        let name = result.attribute("testName").unwrap_or("?");
        match result.attribute("outcome").unwrap_or("") {
            "Passed" => counts.passed += 1,
            "NotExecuted" | "Inconclusive" | "Skipped" => counts.skipped += 1,
            _ => {
                counts.failed += 1;
                let full = match result.attribute("testId").and_then(|id| classes.get(id)) {
                    Some(class) if !name.starts_with(*class) => format!("{class}.{name}"),
                    _ => name.to_string(),
                };
                let text_of = |tag: &str| {
                    result
                        .descendants()
                        .find(|n| n.has_tag_name(tag))
                        .and_then(|n| n.text())
                        .unwrap_or("")
                        .trim()
                        .to_string()
                };
                failures.push(Failure {
                    name: full,
                    message: text_of("Message"),
                    stack: text_of("StackTrace"),
                });
            }
        }
    }
    Ok(TrxResults { counts, failures })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape NUnit and MSTest write through VSTest: `testName` holds only
    /// the method, the class is in the definition.
    const VSTEST_NUNIT: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<TestRun xmlns="http://microsoft.com/schemas/VisualStudio/TeamTest/2010">
  <Results>
    <UnitTestResult testId="a" testName="Parses" outcome="Passed" />
    <UnitTestResult testId="b" testName="Rejects(&quot;x&quot;)" outcome="Failed">
      <Output>
        <ErrorInfo>
          <Message>  Expected: True
  But was:  False
</Message>
          <StackTrace>   at Shop.Tests.OrderTests.Rejects(String s) in OrderTests.cs:line 12</StackTrace>
        </ErrorInfo>
      </Output>
    </UnitTestResult>
    <UnitTestResult testId="c" testName="Later" outcome="NotExecuted" />
    <UnitTestResult testId="d" testName="Maybe" outcome="Inconclusive" />
    <UnitTestResult testId="e" testName="Hangs" outcome="Timeout" />
  </Results>
  <TestDefinitions>
    <UnitTest id="a" name="Parses"><TestMethod className="Shop.Tests.OrderTests" name="Parses" /></UnitTest>
    <UnitTest id="b" name="Rejects"><TestMethod className="Shop.Tests.OrderTests" name="Rejects" /></UnitTest>
    <UnitTest id="e" name="Hangs"><TestMethod className="Shop.Tests.SlowTests" name="Hangs" /></UnitTest>
  </TestDefinitions>
</TestRun>"#;

    #[test]
    fn method_only_names_get_their_class() {
        let r = parse(VSTEST_NUNIT).unwrap();
        assert_eq!((r.counts.passed, r.counts.failed, r.counts.skipped), (1, 2, 2));
        assert_eq!(r.failures[0].name, "Shop.Tests.OrderTests.Rejects(\"x\")");
        assert_eq!(r.failures[0].message, "Expected: True\n  But was:  False");
        assert!(r.failures[0].stack.starts_with("at Shop.Tests.OrderTests.Rejects"));
        assert_eq!(r.failures[1].name, "Shop.Tests.SlowTests.Hangs");
        assert_eq!(r.failures[1].message, "");
    }

    #[test]
    fn full_names_are_kept() {
        let xml = r#"<TestRun xmlns="http://microsoft.com/schemas/VisualStudio/TeamTest/2010"><Results>
            <UnitTestResult testId="a" testName="Shop.Tests.CartTests.Empties" outcome="Failed" />
          </Results><TestDefinitions>
            <UnitTest id="a"><TestMethod className="Shop.Tests.CartTests" name="Empties" /></UnitTest>
          </TestDefinitions></TestRun>"#;
        let r = parse(xml).unwrap();
        assert_eq!(r.failures[0].name, "Shop.Tests.CartTests.Empties");
    }

    #[test]
    fn empty_run_and_invalid_file() {
        let r = parse(r#"<TestRun><Results /></TestRun>"#).unwrap();
        assert_eq!(r.counts.total(), 0);
        assert!(parse("<TestRun>").is_err());
    }
}
