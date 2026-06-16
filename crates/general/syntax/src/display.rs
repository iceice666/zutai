use std::fmt;

use crate::ast::{
    Decl, Expr, File, FuncClause, ImportSource, Pattern, PipelineDir, TupleItem, TuplePatternItem,
    TypeExpr, TypeTupleItem,
};

impl fmt::Display for File {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "File")?;
        let n = self.decls.len();
        for (i, decl) in self.decls.iter().enumerate() {
            let is_last = i + 1 == n;
            let prefix = if is_last { "├─" } else { "├─" };
            write_decl(f, decl, prefix, "│  ")?;
        }
        write_expr(f, &self.final_expr, "└─ final: ", "   ")
    }
}

fn write_decl(f: &mut fmt::Formatter<'_>, decl: &Decl, prefix: &str, indent: &str) -> fmt::Result {
    match decl {
        Decl::Inferred { name, value, .. } => {
            writeln!(f, "{prefix} Inferred {name:?}")?;
            write_expr(
                f,
                value,
                &format!("{indent}└─ value: "),
                &format!("{indent}   "),
            )
        }
        Decl::Typed {
            name, ty, value, ..
        } => {
            writeln!(f, "{prefix} Typed {name:?}")?;
            write_type_expr(
                f,
                ty,
                &format!("{indent}├─ type: "),
                &format!("{indent}│  "),
            )?;
            write_expr(
                f,
                value,
                &format!("{indent}└─ value: "),
                &format!("{indent}   "),
            )
        }
        Decl::TypeAlias {
            name, params, ty, ..
        } => {
            let ps: Vec<_> = params.iter().map(|p| p.name.as_str()).collect();
            writeln!(f, "{prefix} TypeAlias {name:?} <{}>", ps.join(", "))?;
            write_type_expr(f, ty, &format!("{indent}└─ ty: "), &format!("{indent}   "))
        }
        Decl::Function {
            name,
            params,
            sig,
            clauses,
            ..
        } => {
            let ps: Vec<_> = params.iter().map(|p| p.name.as_str()).collect();
            writeln!(f, "{prefix} Function {name:?} <{}>", ps.join(", "))?;
            write_type_expr(
                f,
                sig,
                &format!("{indent}├─ sig: "),
                &format!("{indent}│  "),
            )?;
            for clause in clauses {
                write_clause(f, clause, &format!("{indent}├─ "), &format!("{indent}│  "))?;
            }
            Ok(())
        }
        Decl::NoSigFn {
            name,
            patterns,
            body,
            ..
        } => {
            writeln!(f, "{prefix} NoSigFn {name:?}")?;
            for p in patterns {
                write_pattern(f, p, &format!("{indent}├─ pat: "), &format!("{indent}│  "))?;
            }
            write_expr(
                f,
                body,
                &format!("{indent}└─ body: "),
                &format!("{indent}   "),
            )
        }
    }
}

fn write_clause(
    f: &mut fmt::Formatter<'_>,
    clause: &FuncClause,
    prefix: &str,
    indent: &str,
) -> fmt::Result {
    writeln!(f, "{prefix}Clause")?;
    for p in &clause.patterns {
        write_pattern(f, p, &format!("{indent}├─ pat: "), &format!("{indent}│  "))?;
    }
    if let Some(g) = &clause.guard {
        write_expr(
            f,
            g,
            &format!("{indent}├─ guard: "),
            &format!("{indent}│  "),
        )?;
    }
    write_expr(
        f,
        &clause.body,
        &format!("{indent}└─ body: "),
        &format!("{indent}   "),
    )
}

fn write_expr(f: &mut fmt::Formatter<'_>, expr: &Expr, prefix: &str, indent: &str) -> fmt::Result {
    match expr {
        Expr::True(_) => writeln!(f, "{prefix}true"),
        Expr::False(_) => writeln!(f, "{prefix}false"),
        Expr::Integer { value, .. } => writeln!(f, "{prefix}Int({value})"),
        Expr::Float { value, .. } => writeln!(f, "{prefix}Float({value})"),
        Expr::String { value, .. } => writeln!(f, "{prefix}Str({value:?})"),
        Expr::Atom { name, .. } => writeln!(f, "{prefix}Atom(#{name})"),
        Expr::TaggedValue { tag, payload, .. } => {
            writeln!(f, "{prefix}TaggedValue(#{tag})")?;
            write_expr(f, payload, &format!("{indent}└─ "), &format!("{indent}   "))
        }
        Expr::Ident { name, .. } => writeln!(f, "{prefix}Ident({name})"),
        Expr::Record { fields, .. } => {
            writeln!(f, "{prefix}Record")?;
            for field in fields {
                write_expr(
                    f,
                    &field.value,
                    &format!("{indent}├─ {}: ", field.name),
                    &format!("{indent}│  "),
                )?;
            }
            Ok(())
        }
        Expr::Tuple { items, .. } => {
            writeln!(f, "{prefix}Tuple")?;
            for item in items {
                match item {
                    TupleItem::Named { name, value, .. } => write_expr(
                        f,
                        value,
                        &format!("{indent}├─ {name}="),
                        &format!("{indent}│  "),
                    )?,
                    TupleItem::Positional(e) => {
                        write_expr(f, e, &format!("{indent}├─ "), &format!("{indent}│  "))?
                    }
                }
            }
            Ok(())
        }
        Expr::List { items, .. } => {
            writeln!(f, "{prefix}List")?;
            for item in items {
                write_expr(f, item, &format!("{indent}├─ "), &format!("{indent}│  "))?;
            }
            Ok(())
        }
        Expr::Block {
            bindings, result, ..
        } => {
            writeln!(f, "{prefix}Block")?;
            for b in bindings {
                write_expr(
                    f,
                    &b.value,
                    &format!("{indent}├─ {}: ", b.name),
                    &format!("{indent}│  "),
                )?;
            }
            write_expr(
                f,
                result,
                &format!("{indent}└─ result: "),
                &format!("{indent}   "),
            )
        }
        Expr::Lambda { params, body, .. } => {
            writeln!(f, "{prefix}Lambda")?;
            for p in params {
                write_pattern(
                    f,
                    p,
                    &format!("{indent}├─ param: "),
                    &format!("{indent}│  "),
                )?;
            }
            write_expr(
                f,
                body,
                &format!("{indent}└─ body: "),
                &format!("{indent}   "),
            )
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            writeln!(f, "{prefix}If")?;
            write_expr(
                f,
                cond,
                &format!("{indent}├─ cond: "),
                &format!("{indent}│  "),
            )?;
            write_expr(
                f,
                then_branch,
                &format!("{indent}├─ then: "),
                &format!("{indent}│  "),
            )?;
            write_expr(
                f,
                else_branch,
                &format!("{indent}└─ else: "),
                &format!("{indent}   "),
            )
        }
        Expr::Match {
            scrutinee, arms, ..
        } => {
            writeln!(f, "{prefix}Match")?;
            write_expr(
                f,
                scrutinee,
                &format!("{indent}├─ on: "),
                &format!("{indent}│  "),
            )?;
            for arm in arms {
                write_clause(f, arm, &format!("{indent}├─ "), &format!("{indent}│  "))?;
            }
            Ok(())
        }
        Expr::Import { source, .. } => match source {
            ImportSource::String(s) => writeln!(f, "{prefix}Import({s:?})"),
            ImportSource::Path(p) => writeln!(f, "{prefix}Import({})", p.join(".")),
        },
        Expr::TypeForm { ty, .. } => {
            writeln!(f, "{prefix}TypeForm")?;
            write_type_expr(f, ty, &format!("{indent}└─ "), &format!("{indent}   "))
        }
        Expr::Apply { func, arg, .. } => {
            writeln!(f, "{prefix}Apply")?;
            write_expr(
                f,
                func,
                &format!("{indent}├─ fn: "),
                &format!("{indent}│  "),
            )?;
            write_expr(
                f,
                arg,
                &format!("{indent}└─ arg: "),
                &format!("{indent}   "),
            )
        }
        Expr::Access {
            receiver, field, ..
        } => {
            writeln!(f, "{prefix}Access .{field}")?;
            write_expr(
                f,
                receiver,
                &format!("{indent}└─ "),
                &format!("{indent}   "),
            )
        }
        Expr::OptAccess {
            receiver, field, ..
        } => {
            writeln!(f, "{prefix}OptAccess ?.{field}")?;
            write_expr(
                f,
                receiver,
                &format!("{indent}└─ "),
                &format!("{indent}   "),
            )
        }
        Expr::Binary { op, lhs, rhs, .. } => {
            writeln!(f, "{prefix}Binary({op:?})")?;
            write_expr(f, lhs, &format!("{indent}├─ "), &format!("{indent}│  "))?;
            write_expr(f, rhs, &format!("{indent}└─ "), &format!("{indent}   "))
        }
        Expr::Pipeline { dir, lhs, rhs, .. } => {
            let sym = match dir {
                PipelineDir::Forward => "|>",
                PipelineDir::Backward => "<|",
            };
            writeln!(f, "{prefix}Pipeline({sym})")?;
            write_expr(f, lhs, &format!("{indent}├─ "), &format!("{indent}│  "))?;
            write_expr(f, rhs, &format!("{indent}└─ "), &format!("{indent}   "))
        }
    }
}

fn write_pattern(
    f: &mut fmt::Formatter<'_>,
    pat: &Pattern,
    prefix: &str,
    indent: &str,
) -> fmt::Result {
    match pat {
        Pattern::Wildcard(_) => writeln!(f, "{prefix}_"),
        Pattern::Ident { name, .. } => writeln!(f, "{prefix}Ident({name})"),
        Pattern::True(_) => writeln!(f, "{prefix}true"),
        Pattern::False(_) => writeln!(f, "{prefix}false"),
        Pattern::Integer { value, .. } => writeln!(f, "{prefix}Int({value})"),
        Pattern::Float { value, .. } => writeln!(f, "{prefix}Float({value})"),
        Pattern::String { value, .. } => writeln!(f, "{prefix}Str({value:?})"),
        Pattern::Atom { name, .. } => writeln!(f, "{prefix}Atom(#{name})"),
        Pattern::TaggedValue { tag, payload, .. } => {
            writeln!(f, "{prefix}TaggedPat(#{tag})")?;
            for field in payload {
                write_pattern(
                    f,
                    &field.pattern,
                    &format!("{indent}├─ {}=", field.name),
                    &format!("{indent}│  "),
                )?;
            }
            Ok(())
        }
        Pattern::Tuple { items, .. } => {
            writeln!(f, "{prefix}TuplePat")?;
            for item in items {
                match item {
                    TuplePatternItem::Named { name, pattern, .. } => {
                        write_pattern(
                            f,
                            pattern,
                            &format!("{indent}├─ {name}="),
                            &format!("{indent}│  "),
                        )?;
                    }
                    TuplePatternItem::Positional(p) => {
                        write_pattern(f, p, &format!("{indent}├─ "), &format!("{indent}│  "))?;
                    }
                }
            }
            Ok(())
        }
        Pattern::Record { fields, .. } => {
            writeln!(f, "{prefix}RecordPat")?;
            for field in fields {
                write_pattern(
                    f,
                    &field.pattern,
                    &format!("{indent}├─ {}=", field.name),
                    &format!("{indent}│  "),
                )?;
            }
            Ok(())
        }
    }
}

fn write_type_expr(
    f: &mut fmt::Formatter<'_>,
    ty: &TypeExpr,
    prefix: &str,
    indent: &str,
) -> fmt::Result {
    match ty {
        TypeExpr::Ident { name, .. } => writeln!(f, "{prefix}TyIdent({name})"),
        TypeExpr::Atom { name, .. } => writeln!(f, "{prefix}TyAtom(#{name})"),
        TypeExpr::True(_) => writeln!(f, "{prefix}TyTrue"),
        TypeExpr::False(_) => writeln!(f, "{prefix}TyFalse"),
        TypeExpr::Record { fields, .. } => {
            writeln!(f, "{prefix}TyRecord")?;
            for field in fields {
                let opt = if field.optional { "?" } else { "" };
                write_type_expr(
                    f,
                    &field.ty,
                    &format!("{indent}├─ {}{opt}: ", field.name),
                    &format!("{indent}│  "),
                )?;
            }
            Ok(())
        }
        TypeExpr::Union { variants, .. } => {
            writeln!(f, "{prefix}TyUnion")?;
            for v in variants {
                if let Some(fields) = &v.payload {
                    writeln!(f, "{indent}├─ {}:", v.name)?;
                    for field in fields {
                        let opt = if field.optional { "?" } else { "" };
                        write_type_expr(
                            f,
                            &field.ty,
                            &format!("{indent}│  ├─ {}{opt}: ", field.name),
                            &format!("{indent}│  │  "),
                        )?;
                    }
                } else {
                    writeln!(f, "{indent}├─ {}", v.name)?;
                }
            }
            Ok(())
        }
        TypeExpr::Tuple { items, .. } => {
            writeln!(f, "{prefix}TyTuple")?;
            for item in items {
                match item {
                    TypeTupleItem::Named { name, ty, .. } => write_type_expr(
                        f,
                        ty,
                        &format!("{indent}├─ {name}: "),
                        &format!("{indent}│  "),
                    )?,
                    TypeTupleItem::Positional(t) => {
                        write_type_expr(f, t, &format!("{indent}├─ "), &format!("{indent}│  "))?
                    }
                }
            }
            Ok(())
        }
        TypeExpr::Optional { inner, .. } => {
            writeln!(f, "{prefix}TyOptional")?;
            write_type_expr(f, inner, &format!("{indent}└─ "), &format!("{indent}   "))
        }
        TypeExpr::Arrow { from, to, .. } => {
            writeln!(f, "{prefix}TyArrow")?;
            write_type_expr(
                f,
                from,
                &format!("{indent}├─ from: "),
                &format!("{indent}│  "),
            )?;
            write_type_expr(f, to, &format!("{indent}└─ to: "), &format!("{indent}   "))
        }
        TypeExpr::Apply { func, arg, .. } => {
            writeln!(f, "{prefix}TyApply")?;
            write_type_expr(
                f,
                func,
                &format!("{indent}├─ fn: "),
                &format!("{indent}│  "),
            )?;
            write_type_expr(
                f,
                arg,
                &format!("{indent}└─ arg: "),
                &format!("{indent}   "),
            )
        }
        TypeExpr::Access {
            receiver, field, ..
        } => {
            writeln!(f, "{prefix}TyAccess .{field}")?;
            write_type_expr(
                f,
                receiver,
                &format!("{indent}└─ "),
                &format!("{indent}   "),
            )
        }
        TypeExpr::ExprEscape(e) => {
            writeln!(f, "{prefix}TyExprEscape")?;
            write_expr(f, e, &format!("{indent}└─ "), &format!("{indent}   "))
        }
    }
}
