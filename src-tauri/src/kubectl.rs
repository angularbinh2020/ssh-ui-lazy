use serde::Serialize;

use crate::error::AppError;

pub const SEARCH_LIMIT: usize = 200;

/// Escape `'` trong nháy đơn: `a'b` → `a'\''b`
pub fn escape_single_quote(s: &str) -> String {
    s.replace('\'', r"'\''")
}

pub fn search_command(context: &str, keyword: &str, request_id: &str) -> String {
    let kw = escape_single_quote(keyword);
    format!(
        "kubectl get pod -A --context '{ctx}' 2>&1 | grep -i -F -- '{kw}' | grep -v '^NAMESPACE' | head -n {limit}; echo __LAZY_SEARCH_{rid}__",
        ctx = escape_single_quote(context),
        kw = kw,
        limit = SEARCH_LIMIT,
        rid = request_id,
    )
}

pub fn containers_command(pod: &str, ns: &str, context: &str, marker_tag: &str) -> String {
    format!(
        "kubectl get pod '{pod}' -n '{ns}' --context '{ctx}' -o jsonpath='{{range .spec.containers[*]}}{{.name}}{{\"\\n\"}}{{end}}' 2>&1; echo __LAZY_CT_{tag}__",
        pod = escape_single_quote(pod),
        ns = escape_single_quote(ns),
        ctx = escape_single_quote(context),
        tag = marker_tag,
    )
}

pub fn log_command(
    ns: &str,
    pod: &str,
    container: &str,
    tail: u32,
    timestamps: bool,
    since: Option<&str>,
    context: &str,
) -> String {
    let mut cmd = format!(
        "kubectl -n '{ns}' --context '{ctx}' logs -f -c '{c}' --tail={tail}",
        ns = escape_single_quote(ns),
        ctx = escape_single_quote(context),
        c = escape_single_quote(container),
        tail = tail,
    );
    if timestamps {
        cmd.push_str(" --timestamps");
    }
    if let Some(since) = since {
        if !since.trim().is_empty() {
            cmd.push_str(&format!(" --since={}", escape_single_quote(since.trim())));
        }
    }
    cmd.push_str(&format!(" '{}' 2>&1", escape_single_quote(pod)));
    cmd
}

pub fn get_contexts_command(marker_tag: &str) -> String {
    format!("kubectl config get-contexts -o name 2>&1; echo __LAZY_CTX_{marker_tag}__")
}

pub fn use_context_check_command(context: &str, tag: &str) -> String {
    format!(
        "kubectl config use-context '{ctx}' >/dev/null 2>&1 && echo __LAZY_S5_OK_{tag}__ || echo __LAZY_S5_NO_{tag}__",
        ctx = escape_single_quote(context),
        tag = tag,
    )
}

pub fn kubectl_check_command(tag: &str) -> String {
    format!(
        "command -v kubectl >/dev/null 2>&1 && echo __LAZY_S4_OK_{tag}__ || echo __LAZY_S4_NO_{tag}__",
        tag = tag,
    )
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PodRow {
    pub namespace: String,
    pub pod: String,
    pub ready: String,
    pub status: String,
    pub restarts: String,
    pub age: String,
}

/// Cắt output tại dòng marker (marker phải nằm ở đầu dòng — bỏ dòng echo lệnh).
pub fn cut_at_marker<'a>(output: &'a str, marker: &str) -> &'a str {
    let bytes = output.as_bytes();
    let token = marker.as_bytes();
    if token.is_empty() {
        return output;
    }
    let mut from = 0usize;
    while let Some(pos) = bytes[from..]
        .windows(token.len())
        .position(|w| w == token)
        .map(|p| p + from)
    {
        let at_start = pos == 0 || matches!(bytes[pos - 1], b'\n' | b'\r');
        if at_start {
            return &output[..pos];
        }
        from = pos + 1;
    }
    output
}

/// Parse bảng `kubectl get pod -A` đã qua grep: bỏ header NAMESPACE,
/// tách cột bằng >=2 khoảng trắng. Trả về (rows, bị_cắt_ở_limit).
pub fn parse_pod_rows(output: &str) -> (Vec<PodRow>, bool) {
    let re = regex::Regex::new(r"\s{2,}").expect("regex hop le");
    let mut rows = Vec::new();
    for line in output.lines() {
        let line = line.trim_end_matches('\r').trim();
        // lọc dòng echo lệnh do shell in lại (`$ kubectl ... ; echo __LAZY_...__`)
        if line.is_empty()
            || line.starts_with("NAMESPACE")
            || line.contains("__LAZY_")
            || line.contains("kubectl ")
        {
            continue;
        }
        let parts: Vec<&str> = re.split(line).filter(|p| !p.is_empty()).collect();
        if parts.len() >= 6 {
            rows.push(PodRow {
                namespace: parts[0].to_string(),
                pod: parts[1].to_string(),
                ready: parts[2].to_string(),
                status: parts[3].to_string(),
                restarts: parts[4].to_string(),
                age: parts[5..].join(" "),
            });
        } else if parts.len() == 1 {
            // có thể cột dính nhau (grep cắt) — thử split theo khoảng trắng đơn
            let tokens: Vec<&str> = line.split_whitespace().collect();
            if tokens.len() >= 6 {
                rows.push(PodRow {
                    namespace: tokens[0].to_string(),
                    pod: tokens[1].to_string(),
                    ready: tokens[2].to_string(),
                    status: tokens[3].to_string(),
                    restarts: tokens[4].to_string(),
                    age: tokens[5..].join(" "),
                });
            }
        }
    }
    let truncated = rows.len() >= SEARCH_LIMIT;
    (rows, truncated)
}

/// Parse danh sách container (1 tên / dòng). Lỗi NotFound → AppError.
pub fn parse_container_names(output: &str, pod: &str, ns: &str) -> Result<Vec<String>, AppError> {
    if output.contains("NotFound") || output.contains("not found") || output.contains("No resources found") {
        return Err(AppError::PodNotFound { pod: pod.into(), ns: ns.into() });
    }
    let mut names = Vec::new();
    for line in cut_at_marker(output, "__LAZY_CT_").lines() {
        let name = line.trim_end_matches('\r').trim();
        if name.is_empty()
            || name.contains("__LAZY_")
            || name.contains("kubectl ")
            || name.starts_with("error:")
            || name.starts_with("Error")
        {
            continue;
        }
        names.push(name.to_string());
    }
    Ok(names)
}

/// Parse danh sách context (`kubectl config get-contexts -o name`).
pub fn parse_contexts(output: &str) -> Vec<String> {
    cut_at_marker(output, "__LAZY_CTX_")
        .lines()
        .map(|l| l.trim_end_matches('\r').trim())
        .filter(|l| {
            !l.is_empty()
                && !l.contains("__LAZY_")
                && !l.contains("kubectl ")
                && !l.contains("echo ")
                && !l.starts_with("error:")
                && !l.starts_with("Error")
                && !l.contains("No contexts")
        })
        .map(|l| l.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape() {
        assert_eq!(escape_single_quote("nginx"), "nginx");
        assert_eq!(escape_single_quote("it's"), r"it'\''s");
        assert_eq!(escape_single_quote("a'b'c"), r"a'\''b'\''c");
    }

    #[test]
    fn search_cmd_shape() {
        let cmd = search_command("staging-cluster", "web!@'", "req1");
        assert!(cmd.contains("kubectl get pod -A --context 'staging-cluster' 2>&1"));
        assert!(cmd.contains(r"grep -i -F -- 'web!@'\'''"));
        assert!(cmd.contains("| grep -v '^NAMESPACE'"));
        assert!(cmd.contains("| head -n 200"));
        assert!(cmd.ends_with("echo __LAZY_SEARCH_req1__"));
    }

    #[test]
    fn log_cmd_shape() {
        let cmd = log_command("ns1", "pod-1", "app", 500, true, Some("1h"), "ctx");
        assert_eq!(
            cmd,
            "kubectl -n 'ns1' --context 'ctx' logs -f -c 'app' --tail=500 --timestamps --since=1h 'pod-1' 2>&1"
        );
        let cmd2 = log_command("ns1", "pod-1", "app", 100, false, None, "ctx");
        assert_eq!(
            cmd2,
            "kubectl -n 'ns1' --context 'ctx' logs -f -c 'app' --tail=100 'pod-1' 2>&1"
        );
    }

    #[test]
    fn parse_rows() {
        let out = "NAMESPACE     NAME                READY   STATUS    RESTARTS   AGE\n\
                   kube-system   coredns-abc         1/1     Running   0          5d2h\n\
                   prod          web-1               0/1     Pending   2          3m\n\
                   \n";
        let (rows, truncated) = parse_pod_rows(out);
        assert_eq!(rows.len(), 2);
        assert!(!truncated);
        assert_eq!(rows[0].namespace, "kube-system");
        assert_eq!(rows[0].pod, "coredns-abc");
        assert_eq!(rows[0].ready, "1/1");
        assert_eq!(rows[0].status, "Running");
        assert_eq!(rows[0].restarts, "0");
        assert_eq!(rows[0].age, "5d2h");
        assert_eq!(rows[1].status, "Pending");
        assert_eq!(rows[1].restarts, "2");
    }

    #[test]
    fn parse_rows_truncated_at_200() {
        let mut out = String::from("NAMESPACE NAME READY STATUS RESTARTS AGE\n");
        for i in 0..SEARCH_LIMIT {
            out.push_str(&format!("ns pod-{i} 1/1 Running 0 1d\n"));
        }
        let (rows, truncated) = parse_pod_rows(&out);
        assert_eq!(rows.len(), SEARCH_LIMIT);
        assert!(truncated);
    }

    #[test]
    fn parse_rows_skips_echo_line() {
        // dòng echo lệnh do shell in lại (khoảng trắng đơn) → không được thành row
        let out = "gitlab-runner@host:~$ kubectl get pod -A --context 'c' 2>&1 | grep -i -F -- 'web' | grep -v '^NAMESPACE' | head -n 200; echo __LAZY_SEARCH_r1__\n\
                   prod          web-1               0/1     Pending   2          3m\n";
        let (rows, truncated) = parse_pod_rows(out);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].pod, "web-1");
        assert!(!truncated);
    }

    #[test]
    fn parse_rows_dense_single_space() {
        let out = "ns pod-1 1/1 Running 0 1d\n";
        let (rows, _) = parse_pod_rows(out);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].namespace, "ns");
        assert_eq!(rows[0].age, "1d");
    }

    #[test]
    fn cut_marker() {
        let out = "kubectl get pod -A --context 'c' 2>&1 | grep -i -F -- 'w' | head -n 200; echo __LAZY_SEARCH_r1__\n__LAZY_SEARCH_r1__\n";
        let cut = cut_at_marker(out, "__LAZY_SEARCH_r1__");
        // cắt tại dòng marker OUTPUT (đầu dòng) — dòng echo lệnh được giữ lại
        assert!(cut.starts_with("kubectl"));
        assert!(cut.trim_end().ends_with("echo __LAZY_SEARCH_r1__"));
        assert!(!cut.contains("\n__LAZY_SEARCH_r1__\n"));
        // không có marker → trả nguyên
        assert_eq!(cut_at_marker("plain", "__LAZY_X__"), "plain");
    }

    #[test]
    fn parse_containers() {
        let out = "app\nsidecar\n__LAZY_CT_m1__\n";
        let names = parse_container_names(out, "pod", "ns").unwrap();
        assert_eq!(names, vec!["app", "sidecar"]);

        let err = parse_container_names("Error from server (NotFound): pods \"x\" not found", "x", "ns");
        assert!(err.is_err());

        assert!(parse_container_names("app\n__LAZY_CT_m1__\n", "pod", "ns").is_ok());
    }

    #[test]
    fn parse_contexts_list() {
        let out = "kubectl config get-contexts -o name 2>&1; echo __LAZY_CTX_m__\nctx-a\nctx-b\n__LAZY_CTX_m__\n";
        assert_eq!(parse_contexts(out), vec!["ctx-a", "ctx-b"]);
    }
}
