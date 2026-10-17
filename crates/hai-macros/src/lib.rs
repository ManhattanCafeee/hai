use proc_macro::TokenStream;
use quote::quote;
use syn::{
    Attribute, Ident, ItemStruct, Meta, Token,
    parse::{Parse, ParseStream},
    parse_macro_input,
};

/// Attribute syntax: `args = none` / `args = Type` / `speaks` / `ends_turn`, comma-separated so
/// they compose (for example `args = FooArgs, ends_turn`). The generated implementation always
/// uses `crate::` paths, so a call site needs neither the trait nor the error type imported (this
/// crate is workspace-internal).
#[derive(Default)]
struct ToolArgs {
    args_type: Option<Option<Box<syn::Type>>>,
    speaks: bool,
    ends_turn: bool,
}

impl Parse for ToolArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut parsed = ToolArgs::default();
        while !input.is_empty() {
            let ident: Ident = input.parse()?;
            let key = ident.to_string();
            match key.as_str() {
                "args" => {
                    let _: Token![=] = input.parse()?;
                    if input.peek(Ident) {
                        let lookahead = input.fork();
                        let id: Ident = lookahead.parse()?;
                        if id == "none" {
                            input.parse::<Ident>()?;
                            parsed.args_type = Some(None);
                        } else {
                            parsed.args_type = Some(Some(Box::new(input.parse()?)));
                        }
                    } else {
                        parsed.args_type = Some(Some(Box::new(input.parse()?)));
                    }
                }
                "speaks" => parsed.speaks = true,
                "ends_turn" => parsed.ends_turn = true,
                other => {
                    return Err(syn::Error::new(
                        ident.span(),
                        format!("unknown tool attribute `{other}`"),
                    ));
                }
            }
            if input.is_empty() {
                break;
            }
            let _: Token![,] = input.parse()?;
        }
        Ok(parsed)
    }
}
/// The tool description is every line of the struct's doc comment (trimmed per line, blank lines
/// dropped, line breaks kept).
fn doc_comment(attrs: &[Attribute]) -> String {
    let mut lines: Vec<String> = Vec::new();
    for attr in attrs {
        if !attr.path().is_ident("doc") {
            continue;
        }
        let Meta::NameValue(nv) = &attr.meta else {
            continue;
        };
        let syn::Expr::Lit(expr) = &nv.value else {
            continue;
        };
        let syn::Lit::Str(s) = &expr.lit else {
            continue;
        };
        let text = s.value().trim().to_string();
        if !text.is_empty() {
            lines.push(text);
        }
    }
    lines.join("\n")
}

fn camel_to_snake(name: &str) -> String {
    let mut result = String::new();
    for (i, c) in name.char_indices() {
        if c.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(c.to_ascii_lowercase());
        } else {
            result.push(c);
        }
    }
    result
}

#[proc_macro_attribute]
pub fn tool(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemStruct);
    let struct_name = input.ident.clone();
    let name_lit = camel_to_snake(&struct_name.to_string());
    let description = doc_comment(&input.attrs);
    let parsed = match syn::parse::<ToolArgs>(attr) {
        Ok(parsed) => parsed,
        Err(e) => return e.to_compile_error().into(),
    };
    let metadata = metadata_tokens(&parsed);

    match parsed.args_type {
        Some(None) => {
            let expanded = quote! {
                #[derive(::std::fmt::Debug)]
                #input

                #[::async_trait::async_trait]
                impl crate::agentcore::tool::AgentTool for #struct_name {
                    fn name(&self) -> &str { #name_lit }
                    fn description(&self) -> &str { #description }
                    fn schema(&self) -> ::std::option::Option<::serde_json::Value> { None }
                    async fn execute(
                        &self,
                        _args: ::serde_json::Value,
                    ) -> ::std::result::Result<
                        ::serde_json::Value,
                        crate::agentcore::tool::ToolError,
                    > {
                        self.exec().await
                    }
                    #metadata
                }
            };
            TokenStream::from(expanded)
        }
        Some(Some(args_type)) => expand_with_args(
            &struct_name,
            name_lit,
            description,
            &metadata,
            input,
            *args_type,
        ),
        None => {
            let args_name_str = format!("{}Args", struct_name);
            let args_ident = Ident::new(&args_name_str, struct_name.span());
            let args_type = syn::parse2::<syn::Type>(quote!(#args_ident))
                .expect("failed to construct args type");
            expand_with_args(
                &struct_name,
                name_lit,
                description,
                &metadata,
                input,
                args_type,
            )
        }
    }
}

/// Emits only the metadata methods that were declared explicitly; the rest keep the trait default.
fn metadata_tokens(parsed: &ToolArgs) -> proc_macro2::TokenStream {
    let mut tokens = proc_macro2::TokenStream::new();
    if parsed.speaks {
        tokens.extend(quote! {
            fn speaks(&self) -> bool { true }
        });
    }
    if parsed.ends_turn {
        tokens.extend(quote! {
            fn ends_turn(&self) -> bool { true }
        });
    }
    tokens
}

fn expand_with_args(
    struct_name: &Ident,
    name_lit: String,
    description: String,
    metadata: &proc_macro2::TokenStream,
    input: ItemStruct,
    args_type: syn::Type,
) -> TokenStream {
    let expanded = quote! {
        #[derive(::std::fmt::Debug)]
        #input

        #[::async_trait::async_trait]
        impl crate::agentcore::tool::AgentTool for #struct_name {
            fn name(&self) -> &str { #name_lit }
            fn description(&self) -> &str { #description }
            fn schema(&self) -> ::std::option::Option<::serde_json::Value> {
                ::std::option::Option::Some(
                    crate::agentcore::tool::schema_of::<#args_type>()
                )
            }
            async fn execute(
                &self,
                args: ::serde_json::Value,
            ) -> ::std::result::Result<
                ::serde_json::Value,
                crate::agentcore::tool::ToolError,
            > {
                let typed: #args_type = ::serde_json::from_value(args)?;
                self.exec(typed).await
            }
            #metadata
        }
    };
    TokenStream::from(expanded)
}
