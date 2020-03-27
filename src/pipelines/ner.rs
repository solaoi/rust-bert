// Copyright 2019-present, the HuggingFace Inc. team, The Google AI Language Team and Facebook, Inc.
// Copyright 2019 Guillaume Becquin
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//     http://www.apache.org/licenses/LICENSE-2.0
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! # Named Entity Recognition pipeline
//! Extracts entities (Person, Location, Organization, Miscellaneous) from text.
//! BERT cased large model finetuned on CoNNL03, contributed by the [MDZ Digital Library team at the Bavarian State Library](https://github.com/dbmdz)
//! All resources for this model can be downloaded using the Python utility script included in this repository.
//! 1. Set-up a Python virtual environment and install dependencies (in ./requirements.txt)
//! 2. Run the conversion script python /utils/download-dependencies_bert_ner.py.
//! The dependencies will be downloaded to the user's home directory, under ~/rustbert/bert-ner
//!
//! ```no_run
//!# use std::path::PathBuf;
//!# use tch::Device;
//! use rust_bert::pipelines::ner::NERModel;
//!# fn main() -> failure::Fallible<()> {
//!# let mut home: PathBuf = dirs::home_dir().unwrap();
//!# home.push("rustbert");
//!# home.push("bert-ner");
//!# let config_path = &home.as_path().join("config.json");
//!# let vocab_path = &home.as_path().join("vocab.txt");
//!# let weights_path = &home.as_path().join("model.ot");
//! let device = Device::cuda_if_available();
//! let ner_model = NERModel::new(vocab_path,
//!                               config_path,
//!                               weights_path, device)?;
//! let input = [
//!     "My name is Amy. I live in Paris.",
//!     "Paris is a city in France."
//! ];
//! let output = ner_model.predict(&input);
//!# Ok(())
//!# }
//! ```
//! Output: \
//! ```no_run
//!# use rust_bert::pipelines::question_answering::Answer;
//!# use rust_bert::pipelines::ner::Entity;
//!# let output =
//! [
//!    Entity { word: String::from("Amy"), score: 0.9986, label: String::from("I-PER") },
//!    Entity { word: String::from("Paris"), score: 0.9985, label: String::from("I-LOC") },
//!    Entity { word: String::from("Paris"), score: 0.9988, label: String::from("I-LOC") },
//!    Entity { word: String::from("France"), score: 0.9993, label: String::from("I-LOC") },
//! ]
//!# ;
//! ```

use rust_tokenizers::bert_tokenizer::BertTokenizer;
use std::path::Path;
use tch::nn::VarStore;
use rust_tokenizers::preprocessing::tokenizer::base_tokenizer::{TruncationStrategy, MultiThreadedTokenizer};
use std::collections::HashMap;
use tch::{Tensor, no_grad, Device};
use tch::kind::Kind::Float;
use crate::bert::{BertForTokenClassification, BertConfig};
use crate::Config;


#[derive(Debug)]
/// # Entity generated by a `NERModel`
pub struct Entity {
    /// String representation of the Entity
    pub word: String,
    /// Confidence score
    pub score: f64,
    /// Entity label (e.g. ORG, LOC...)
    pub label: String,
}

/// # NERModel to extract named entities
pub struct NERModel {
    tokenizer: BertTokenizer,
    bert_sequence_classifier: BertForTokenClassification,
    label_mapping: HashMap<i64, String>,
    var_store: VarStore,
}

impl NERModel {
    /// Build a new `NERModel`
    ///
    /// # Arguments
    ///
    /// * `vocab_path` - Path to the model vocabulary, expected to have a structure following the [Transformers library](https://github.com/huggingface/transformers) convention
    /// * `config_path` - Path to the model configuration, expected to have a structure following the [Transformers library](https://github.com/huggingface/transformers) convention
    /// * `weights_path` - Path to the model weight files. These need to be converted form the `.bin` to `.ot` format using the utility script provided.
    /// * `device` - Device to run the model on, e.g. `Device::Cpu` or `Device::Cuda(0)`
    ///
    /// # Example
    ///
    /// ```no_run
    ///# fn main() -> failure::Fallible<()> {
    /// use tch::Device;
    /// use std::path::{Path, PathBuf};
    /// use rust_bert::pipelines::ner::NERModel;
    ///
    /// let mut home: PathBuf = dirs::home_dir().unwrap();
    /// let config_path = &home.as_path().join("config.json");
    /// let vocab_path = &home.as_path().join("vocab.txt");
    /// let weights_path = &home.as_path().join("model.ot");
    /// let device = Device::Cpu;
    /// let ner_model =  NERModel::new(vocab_path,
    ///                                config_path,
    ///                                weights_path,
    ///                                device)?;
    ///# Ok(())
    ///# }
    /// ```
    ///
    pub fn new(vocab_path: &Path, config_path: &Path, weights_path: &Path, device: Device)
               -> failure::Fallible<NERModel> {
        let tokenizer = BertTokenizer::from_file(vocab_path.to_str().unwrap(), false);
        let mut var_store = VarStore::new(device);
        let config = BertConfig::from_file(config_path);
        let bert_sequence_classifier = BertForTokenClassification::new(&var_store.root(), &config);
        let label_mapping = config.id2label.expect("No label dictionary (id2label) provided in configuration file");
        var_store.load(weights_path)?;
        Ok(NERModel { tokenizer, bert_sequence_classifier, label_mapping, var_store })
    }

    fn prepare_for_model(&self, input: Vec<&str>) -> Tensor {
        let tokenized_input = self.tokenizer.encode_list(input.to_vec(),
                                                         128,
                                                         &TruncationStrategy::LongestFirst,
                                                         0);
        let max_len = tokenized_input.iter().map(|input| input.token_ids.len()).max().unwrap();
        let tokenized_input = tokenized_input.
            iter().
            map(|input| input.token_ids.clone()).
            map(|mut input| {
                input.extend(vec![0; max_len - input.len()]);
                input
            }).
            map(|input|
                Tensor::of_slice(&(input))).
            collect::<Vec<_>>();
        Tensor::stack(tokenized_input.as_slice(), 0).to(self.var_store.device())
    }

    /// Extract entities from a text
    ///
    /// # Arguments
    ///
    /// * `input` - `&[&str]` Array of texts to extract entities from.
    ///
    /// # Returns
    ///
    /// * `Vec<Entity>` containing extracted entities
    ///
    /// # Example
    ///
    /// ```no_run
    ///# fn main() -> failure::Fallible<()> {
    ///# use tch::Device;
    ///# use std::path::{Path, PathBuf};
    ///# use rust_bert::pipelines::ner::NERModel;
    ///#
    ///# let mut home: PathBuf = dirs::home_dir().unwrap();
    ///# let config_path = &home.as_path().join("config.json");
    ///# let vocab_path = &home.as_path().join("vocab.txt");
    ///# let weights_path = &home.as_path().join("model.ot");
    ///# let device = Device::Cpu;
    /// let ner_model =  NERModel::new(vocab_path,
    ///                                config_path,
    ///                                weights_path,
    ///                                device)?;
    /// let input = [
    ///     "My name is Amy. I live in Paris.",
    ///     "Paris is a city in France."
    /// ];
    /// let output = ner_model.predict(&input);
    ///# Ok(())
    ///# }
    /// ```
    ///
    pub fn predict(&self, input: &[&str]) -> Vec<Entity> {
        let input_tensor = self.prepare_for_model(input.to_vec());
        let (output, _, _) = no_grad(|| {
            self.bert_sequence_classifier
                .forward_t(Some(input_tensor.copy()),
                           None,
                           None,
                           None,
                           None,
                           false)
        });
        let output = output.detach().to(Device::Cpu);
        let score: Tensor = output.exp() / output.exp().sum1(&[-1], true, Float);
        let labels_idx = &score.argmax(-1, true);

        let mut entities: Vec<Entity> = vec!();
        for sentence_idx in 0..labels_idx.size()[0] {
            let labels = labels_idx.get(sentence_idx);
            for position_idx in 0..labels.size()[0] {
                let label = labels.int64_value(&[position_idx]);
                if label != 0 {
                    entities.push(Entity {
                        word: rust_tokenizers::preprocessing::tokenizer::base_tokenizer::Tokenizer::decode(&self.tokenizer, vec!(input_tensor.int64_value(&[sentence_idx, position_idx])), true, true),
                        score: score.double_value(&[sentence_idx, position_idx, label]),
                        label: self.label_mapping.get(&label).expect("Index out of vocabulary bounds.").to_owned(),
                    });
                }
            }
        }
        entities
    }
}