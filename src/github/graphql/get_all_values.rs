use url::Url;

use crate::github::graphql::{github_schema::github_schema as schema, types::Html};

#[derive(cynic::QueryVariables)]
pub struct GetAllValuesVariables<'a> {
    pub owner: &'a str,
    pub name: &'a str,
    pub tag_name: &'a str,
}

#[derive(cynic::QueryFragment)]
pub struct Tree {
    #[cynic(flatten)]
    pub entries: Vec<TreeEntry>,
}

#[derive(cynic::QueryFragment)]
pub struct TreeEntry {
    pub name: String,
    #[cynic(rename = "type")]
    pub type_: String,
}

#[derive(cynic::QueryFragment)]
#[cynic(graphql_type = "Query", variables = "GetAllValuesVariables")]
pub struct GetAllValues {
    #[arguments(owner: $owner, name: $name)]
    pub repository: Option<Repository>,
}

/// <https://docs.github.com/graphql/reference/queries#repository>
#[derive(cynic::QueryFragment)]
#[cynic(variables = "GetAllValuesVariables")]
pub struct Repository {
    pub has_issues_enabled: bool,
    pub license_info: Option<License>,
    pub owner: RepositoryOwner,
    #[arguments(expression: "HEAD:")]
    pub object: Option<GetAllValuesGitObject>,
    #[arguments(tagName: $tag_name)]
    pub release: Option<Release>,
    #[cynic(rename = "repositoryTopics")]
    #[arguments(first: 16)]
    pub topics: RepositoryTopicConnection,
    pub url: Url,
}

/// <https://docs.github.com/graphql/reference/objects#repositorytopicconnection>
#[derive(cynic::QueryFragment)]
pub struct RepositoryTopicConnection {
    #[cynic(flatten)]
    pub nodes: Vec<RepositoryTopic>,
}

/// <https://docs.github.com/graphql/reference/objects#repositorytopic>
#[derive(cynic::QueryFragment)]
pub struct RepositoryTopic {
    pub topic: Topic,
}

/// <https://docs.github.com/graphql/reference/objects#topic>
#[derive(cynic::QueryFragment)]
pub struct Topic {
    pub name: String,
}

/// <https://docs.github.com/graphql/reference/objects#release>
#[derive(cynic::QueryFragment)]
pub struct Release {
    #[cynic(rename = "descriptionHTML")]
    pub description_html: Option<Html>,
    pub url: Url,
}

/// <https://docs.github.com/graphql/reference/interfaces#repositoryowner>
#[derive(cynic::QueryFragment)]
pub struct RepositoryOwner {
    pub url: Url,
}

/// <https://docs.github.com/graphql/reference/objects#license>
#[derive(cynic::QueryFragment)]
pub struct License {
    pub key: String,
    #[cynic(rename = "pseudoLicense")]
    pub is_pseudo: bool,
    pub spdx_id: Option<String>,
}

#[derive(cynic::InlineFragments)]
#[cynic(graphql_type = "GitObject")]
pub enum GetAllValuesGitObject {
    Tree(Tree),
    #[cynic(fallback)]
    Unknown,
}

#[cfg(test)]
mod tests {
    use cynic::QueryBuilder;
    use indoc::indoc;

    use crate::github::{
        github_client::{MICROSOFT, WINGET_PKGS},
        graphql::get_all_values::{GetAllValues, GetAllValuesVariables},
    };

    #[test]
    fn get_all_values_output() {
        const GET_ALL_VALUES_QUERY: &str = indoc! {r#"
            query GetAllValues($owner: String!, $name: String!, $tagName: String!) {
              repository(owner: $owner, name: $name) {
                hasIssuesEnabled
                licenseInfo {
                  key
                  pseudoLicense
                  spdxId
                }
                owner {
                  url
                }
                object(expression: "HEAD:") {
                  __typename
                  ... on Tree {
                    entries {
                      name
                      type
                    }
                  }
                }
                release(tagName: $tagName) {
                  descriptionHTML
                  url
                }
                repositoryTopics(first: 16) {
                  nodes {
                    topic {
                      name
                    }
                  }
                }
                url
              }
            }
        "#};

        let operation = GetAllValues::build(GetAllValuesVariables {
            owner: MICROSOFT,
            name: WINGET_PKGS,
            tag_name: "",
        });

        assert_eq!(operation.query, GET_ALL_VALUES_QUERY);
    }
}
