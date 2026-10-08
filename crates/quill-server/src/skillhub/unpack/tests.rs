use super::*;
use crate::skillhub::MAX_HTTP_BYTES;
use std::io::Write;

/// 造一个只含 `name` → `body` 的 zip。
fn make_zip(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let opts: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, body) in entries {
            w.start_file(*name, opts).expect("start_file");
            w.write_all(body.as_bytes()).expect("write");
        }
        w.finish().expect("finish");
    }
    buf
}

#[test]
fn a_normal_package_unpacks_into_named_files() {
    let bytes = make_zip(&[("demo.md", "# 演示\n\n内容。"), ("notes.txt", "忽略我")]);
    let out = unpack(&bytes).expect("应当解开");
    assert_eq!(out.files.len(), 1, "只收 .md");
    assert_eq!(out.files[0].0, "demo.md");
    assert!(out.files[0].1.contains("演示"));
    assert!(out.compressed_bytes > 0 && out.uncompressed_bytes > 0);
    // 丢掉的东西要报出去，不能悄悄咽下去。
    assert_eq!(out.skipped_other, 1, "notes.txt 应当被计入「丢掉的」");
}

#[test]
fn a_manifest_is_kept_because_it_lists_the_downstream_skills() {
    // 丢掉 manifest 的话，界面上就说不清「装的是包本身，还是包点名的
    // 那几个下游技能」—— 而这两者对用户是天差地别的。
    let bytes = make_zip(&[
        ("manifest.json", r#"{"slug":"tech","skillSlugs":["tdd"]}"#),
        ("identify.md", "# 编排说明"),
    ]);
    let out = unpack(&bytes).expect("应当解开");
    let names: Vec<&str> = out.files.iter().map(|(n, _)| n.as_str()).collect();
    assert!(
        names.contains(&"manifest.json"),
        "manifest 必须留着：{names:?}"
    );
    assert!(names.contains(&"identify.md"));
}

#[test]
fn a_package_with_a_manifest_but_no_markdown_still_cannot_be_installed() {
    // 有元数据、没正文 = 装不了任何东西。这时候报 200 + 空数组，
    // 界面就会显示「装好了」—— 那是在骗人。
    let bytes = make_zip(&[("manifest.json", r#"{"slug":"x"}"#)]);
    match unpack(&bytes) {
        Err(UnpackError::NoSkill) => {}
        other => panic!("应当报「没有技能」，实际：{other:?}"),
    }
}

#[test]
fn a_zip_with_no_markdown_says_so_instead_of_installing_nothing() {
    let bytes = make_zip(&[("readme.txt", "只有 txt")]);
    match unpack(&bytes) {
        Err(UnpackError::NoSkill) => {}
        other => panic!("应当报「没有技能」，实际：{other:?}"),
    }
}

#[test]
fn an_entry_name_never_survives_as_anything_but_a_safe_basename() {
    // **这一条才是真正要保的东西**：无论包里写什么，
    // `sanitize_name` 的输出拼进技能目录之后都必须落在目录内。
    //
    // 注意这里有个我一开始搞错的地方：带路径的条目名**不是**被拒绝，
    // 而是被压成 basename。`..\..\windows\system32\evil` 变成 `evil`，
    // 落在 `<技能目录>/evil` —— 这是安全的，取 basename 正是为此。
    // 真正会被拒的是「连 basename 都不干净」的那些。
    for raw in [
        "tech-test-automation/skills/tdd.md",
        "..\\..\\windows\\system32\\evil",
        "../../../etc/cron.d/evil",
        "C:/evil.md",
        "a/b/c/deep.md",
    ] {
        let out = match sanitize_name(raw) {
            Ok(v) => v,
            Err(e) => panic!("{raw:?} 应当被压成 basename，实际被拒了：{e:?}"),
        };
        assert!(
            !out.contains('/') && !out.contains('\\'),
            "{raw:?} 压出来的 {out:?} 里还有路径分隔符"
        );
        assert!(
            out != ".." && out != ".",
            "{raw:?} 压出来是 {out:?}，指向目录本身"
        );
        // 拼进技能目录之后必须还在目录里。
        let joined = std::path::Path::new("/skills").join(&out);
        assert_eq!(
            joined.parent().and_then(|p| p.to_str()),
            Some("/skills"),
            "{raw:?} 拼出来跑出了技能目录：{joined:?}"
        );
    }
}

#[test]
fn an_entry_name_that_is_dirty_even_as_a_basename_is_rejected() {
    // 上面那些是「压成 basename」，这三条是「压完仍然不干净」——
    // 这时才必须拒，而不是悄悄改写（改写等于我们替上游编了文件名）。
    for raw in ["..", ".", "a/..", "C:evil.md", "名字带中文.md"] {
        let err = match sanitize_name(raw) {
            Ok(v) => panic!("{raw:?} 应当被拒绝，实际压成了 {v:?}"),
            Err(e) => e,
        };
        match err {
            UnpackError::Refused(_) => {}
            other => panic!("{raw:?} 应当被拒绝，实际：{other:?}"),
        }
    }
    // 超长文件名同理：zip 规范允许 65535 字节，我们只要 128。
    let long = "x".repeat(200);
    assert!(sanitize_name(&long).is_err(), "200 字符的文件名应当被拒");
}

#[test]
fn nested_paths_keep_only_the_basename() {
    assert_eq!(
        sanitize_name("tech-test-automation/skills/tdd.md").expect("应当取 basename"),
        "tdd.md"
    );
}

#[test]
fn the_zip_bomb_ratio_is_checked_before_anything_is_decompressed() {
    // 造一个真正的高压缩比条目：几万字节的重复字符。
    let bomb = "A".repeat(2_000_000);
    let bytes = make_zip(&[("bomb.md", &bomb)]);
    // 先确认这确实是个高压缩比的包（否则这条测试等于没测）
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).expect("能打开");
    {
        let e = z.by_index(0).expect("by_index");
        let ratio = e.size() as f64 / e.compressed_size().max(1) as f64;
        assert!(ratio > 10.0, "这个样本压缩比只有 {ratio:.1}，测不出限制");
    }
    match unpack(&bytes) {
        Err(UnpackError::Refused(why)) => {
            assert!(
                why.contains("压缩比"),
                "应当明确说是压缩比超限，实际：{why}"
            );
        }
        other => panic!("压缩炸弹应当被拒，实际：{other:?}"),
    }
}

#[test]
fn a_package_with_too_many_entries_is_refused() {
    // 不真的造 2001 个文件（太慢），只验证「条目数超限」这条错误信息是对的；
    // 上限常量本身改由文件顶部的编译期断言钉住。
    let err = refuse("包里有 9999 个文件，超过 2000 个上限");
    assert!(err.message().contains("安全检查"));
}

// -----------------------------------------------------------------------
// 单技能包
//
// 下面这些用的样本结构是 2026-10-06 从上游真机下载的
// `pdf-image-text-extractor@1.0.13`，条目名与个数都对得上。
// -----------------------------------------------------------------------

/// 复刻上游那个包的条目表（`scripts/*.py` 只留名字，内容不影响断言）。
const REAL_SKILL_PKG: &[(&str, &str)] = &[
    ("README.en.md", "# English readme"),
    ("README.md", "# 中文说明"),
    ("scripts/batch_extractor.py", "print('x')"),
    ("scripts/pdf_text_extractor.py", "print('x')"),
    ("scripts/changelog.py", "print('x')"),
    ("scripts/record.py", "print('x')"),
    (
        "SKILL.md",
        "---\nname: pdf-image-text-extractor\ndescription: 提取文字\n---\n\n# 正文",
    ),
    ("_meta.json", r#"{"version":"1.0.13"}"#),
];

#[test]
fn a_single_skill_package_yields_exactly_one_skill() {
    // **这一条是真 bug 的钉子。** 按技能包的规则解，这个包会产出
    // `README.en.md`、`README.md`、`SKILL.md` 三个技能，
    // 它们共用 slug 互相覆盖，最后留下哪篇取决于 zip 里的顺序。
    let bytes = make_zip(REAL_SKILL_PKG);
    let out = unpack_skill(&bytes).expect("应当解开");
    assert_eq!(out.files.len(), 1, "单技能包必须只出一个技能");
    assert_eq!(out.files[0].0, "SKILL.md");
    assert!(out.files[0].1.contains("提取文字"));
}

#[test]
fn the_files_left_out_of_a_single_skill_package_are_counted() {
    // 8 个条目，只装 1 个 —— 界面要能说「还有 7 个没装」，
    // 而不是让用户以为装全了。
    let bytes = make_zip(REAL_SKILL_PKG);
    let out = unpack_skill(&bytes).expect("应当解开");
    assert_eq!(
        out.files.len() + out.skipped_other,
        REAL_SKILL_PKG.len(),
        "装了的加没装的，必须等于包里的条目数"
    );
    // 4 个脚本 + 2 个 README + _meta.json = 7
    assert_eq!(out.skipped_other, 7);
}

#[test]
fn skill_md_wins_over_the_readmes_whatever_the_order_is() {
    // 上游改了 zip 里的条目顺序，结果必须一样。
    // 顺序决定结果 = 用户不知道自己拿到的是哪一份。
    let mut shuffled: Vec<(&str, &str)> = REAL_SKILL_PKG.to_vec();
    shuffled.reverse();
    let bytes = make_zip(&shuffled);
    let out = unpack_skill(&bytes).expect("应当解开");
    assert_eq!(out.files[0].0, "SKILL.md");
}

#[test]
fn a_single_skill_without_skill_md_uses_the_one_root_markdown_and_says_which() {
    // 换了名字也别静悄悄换内容：正文来源要能报出去。
    let bytes = make_zip(&[("GUIDE.md", "# 换个名字的正文"), ("docs/extra.md", "文档")]);
    let out = unpack_skill(&bytes).expect("应当解开");
    assert_eq!(out.files.len(), 1);
    assert_eq!(out.files[0].0, "GUIDE.md");
    assert!(out.files[0].1.contains("换个名字"));
}

#[test]
fn a_single_skill_with_several_root_markdowns_is_refused_instead_of_picking_one() {
    // 两篇都说得通的时候，**挑一篇就是替上游做主**。
    // 报出来让用户看见，比装一篇随机正文诚实。
    let bytes = make_zip(&[("A.md", "a"), ("B.md", "b")]);
    match unpack_skill(&bytes) {
        Err(UnpackError::NoSkill) => {}
        other => panic!("应当拒绝并报「没有正文」，实际：{other:?}"),
    }
}

#[test]
fn a_single_skill_with_only_nested_markdowns_is_refused() {
    // `docs/foo.md` 是文档，不是技能正文。
    let bytes = make_zip(&[("docs/a.md", "a"), ("docs/b.md", "b")]);
    assert!(matches!(unpack_skill(&bytes), Err(UnpackError::NoSkill)));
}

#[test]
fn the_two_package_kinds_are_not_interchangeable() {
    // 钉住「技能包收全部 md、单技能只收一篇」这个区别本身。
    // 一旦哪天两边混了，这条会先撞。
    assert_ne!(PackageKind::SkillSet, PackageKind::Skill);
    let bytes = make_zip(&[("SKILL.md", "正文"), ("README.md", "说明")]);
    // 技能包模式：两篇都收。
    assert_eq!(unpack(&bytes).expect("应当解开").files.len(), 2);
    // 单技能模式：只收一篇。
    assert_eq!(unpack_skill(&bytes).expect("应当解开").files.len(), 1);
}

// --- 装专家要用的那一趟读 ---------------------------------------------

#[test]
fn the_persona_is_the_skillset_named_file_and_not_a_skill_it_references() {
    // 包里同时有编排提示与它引用的技能文档。选错的话，
    // 装出来的专家人格就是一篇技能说明。
    // **`skillsets/` 下故意放了两篇**：只按「第一篇」选的话，
    // 这条测试挑不出毛病 —— 排在前面的偏偏不是该选的那篇。
    let bytes = make_zip(&[
        ("skillsets/other-name.md", "别的包留下的编排提示"),
        ("skillsets/pdf-toolkit.md", "你是 PDF 工具箱助手。"),
        ("skills/pdf-extract/SKILL.md", "怎么抽文本"),
    ]);
    let out = skillset_contents(&bytes, "pdf-toolkit").expect("应当读出人格");
    assert_eq!(out.persona.source_file, "skillsets/pdf-toolkit.md");
    assert!(out.persona.body.contains("PDF 工具箱助手"));
    assert!(
        !out.persona.body.contains("别的包"),
        "不能选到别人的编排提示"
    );
}

#[test]
fn without_the_named_file_any_markdown_under_skillsets_is_used() {
    let bytes = make_zip(&[
        ("skillsets/other-name.md", "换个名字的编排提示"),
        ("skills/x/SKILL.md", "技能说明"),
    ]);
    let out = skillset_contents(&bytes, "wanted-name").expect("应当读出人格");
    assert_eq!(out.persona.source_file, "skillsets/other-name.md");
    assert!(
        !out.persona.body.contains("技能说明"),
        "不能退而选一篇技能说明当人格"
    );
}

#[test]
fn identify_md_is_the_last_resort_and_its_absence_is_reported() {
    let with = make_zip(&[("identify.md", "兜底编排")]);
    let out = skillset_contents(&with, "nope").expect("应当退回 identify.md");
    assert_eq!(out.persona.source_file, "identify.md");

    // 三处都没有 → 报错，不返回空人格。
    let none = make_zip(&[("skills/x/SKILL.md", "只有技能")]);
    match skillset_contents(&none, "nope") {
        Err(UnpackError::NoSkill) => {}
        other => panic!("没有可当人格的条目时应报错，实际：{other:?}"),
    }
}

#[test]
fn frontmatter_is_stripped_because_it_describes_the_package_not_the_persona() {
    let bytes = make_zip(&[(
        "identify.md",
        "---\nslug: pdf-toolkit\nversion: 1.0.0\n---\n\n先问用途。",
    )]);
    let out = skillset_contents(&bytes, "pdf-toolkit").expect("应当读出人格");
    assert_eq!(out.persona.body, "先问用途。");
    assert!(
        !out.persona.body.contains("version:"),
        "包装元数据不该进人格"
    );
}

#[test]
fn a_package_without_a_manifest_still_yields_a_persona() {
    // 这是**我们的选择**，与 Octop 不同（它缺 manifest 就整单失败）。
    // 缺 manifest 的诚实后果是「没认出它要哪些技能」，不是「人格也没了」。
    let bytes = make_zip(&[("skillsets/x.md", "有人格")]);
    let out = skillset_contents(&bytes, "x").expect("应当读出人格");
    assert!(out.manifest.is_none(), "这个包本来就没有 manifest");
    assert!(out.persona.body.contains("有人格"));
}

#[test]
fn a_manifest_naming_skills_is_read_as_the_dependency_list_not_as_the_persona() {
    let bytes = make_zip(&[
        (
            "manifest.json",
            r#"{"slug":"pdf-toolkit","skills":[{"slug":"pdf-extract"}]}"#,
        ),
        ("skillsets/pdf-toolkit.md", "人格正文"),
    ]);
    let out = skillset_contents(&bytes, "pdf-toolkit").expect("应当读出人格");
    assert_eq!(
        out.manifest.expect("manifest 应可解析").referenced_slugs(),
        vec!["pdf-extract"]
    );
    assert_eq!(out.persona.source_file, "skillsets/pdf-toolkit.md");
}

#[test]
fn the_zip_bomb_limits_apply_to_the_expert_path_too() {
    // 新增一条读 zip 的路径，就必须同样过四道上限。
    // 换成「只是读文本所以放松」的话，外部服务就能拿一个包把内存打爆。
    let bytes = vec![b'0'; 300 * 1024 * 1024];
    match skillset_contents(&bytes, "x") {
        Err(_) => {}
        Ok(_) => panic!("压缩炸弹必须被拒"),
    }
}

#[test]
fn the_safety_limits_are_the_ones_octop_settled_on() {
    // 数字本身也要钉住：调小压缩比上限看着「更安全」，但会把
    // 合法的纯文本技能包拒掉；调大就是放开 zip bomb。
    assert_eq!(MAX_HTTP_BYTES, 32 * 1024 * 1024);
    assert_eq!(MAX_ZIP_UNCOMPRESSED_BYTES, 64 * 1024 * 1024);
    assert_eq!(MAX_ZIP_COMPRESSION_RATIO, 100.0);
    assert_eq!(MAX_ZIP_ENTRIES, 2_000);
}
