use std::{collections::HashMap, hash::Hash, marker::PhantomData, borrow::Borrow};

use ph::fmph::{GOBuildConf, GOConf, GOFunction};

pub struct PerfectMap<K, V> {
    function: ph::fmph::GOFunction,
    values: Vec<V>,
    spooky: PhantomData<K>,
}

impl<KEY: Hash + Sync, VALUE: Hash + Sync> PerfectMap<KEY, VALUE> {
    pub fn from_map_invert<U: Into<VALUE>>(map: HashMap<U, KEY>) -> PerfectMap<KEY, VALUE> {
        let (values, keys): (Vec<_>, Vec<_>) = map.into_iter().unzip();

        PerfectMap::new(&keys, values)
    }
}

impl<K: Hash + Sync, V> PerfectMap<K, V> {
    pub fn from_map<U: Into<V>>(map: HashMap<K, U>) -> PerfectMap<K, V> {
        let (keys, values): (Vec<_>, Vec<_>) = map.into_iter().unzip();

        PerfectMap::new(&keys, values)
    }

    pub fn new<U: Into<V>>(keys: &[K], values: Vec<U>) -> PerfectMap<K, V> {
        assert!(keys.len() == values.len());

        let hasher = GOFunction::from_slice_with_conf(
            &keys,
            GOBuildConf::with_lsize(GOConf::default(), 300),
        );

        let map_len = values.len();
        let mut reordered_vals = Vec::with_capacity(map_len);

        for (k, v) in keys.into_iter().zip(values.into_iter()) {
            let new_idx = hasher.get(&k).unwrap() as usize;
            reordered_vals.spare_capacity_mut()[new_idx].write(v.into());
        }

        unsafe {
            reordered_vals.set_len(map_len);
        }

        PerfectMap {
            function: hasher,
            values: reordered_vals,
            spooky: PhantomData,
        }
    }

    pub fn get<Q>(&self, key: &Q) -> Option<&V> where K: Borrow<Q>, Q: Hash + ?Sized  {
        self.function
            .get(key)
            .and_then(|v| self.values.get(v as usize))
    }
}

#[cfg(feature = "serde")]
impl<K, V: serde::Serialize> serde::Serialize for PerfectMap<K,V> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer {
        use serde::ser::{SerializeStruct, Error};

        let mut state = serializer.serialize_struct("PerfectMap", 2)?;
        state.serialize_field("values", &self.values)?;

        let mut hasher_bytes = Vec::with_capacity(self.function.write_bytes());
        self.function.write(&mut hasher_bytes).map_err(|_| S::Error::custom("couldn't write hash function"))?; 
        state.serialize_field("function", &serde_bytes::ByteBuf::from(hasher_bytes))?;
        state.end()
    }
}


#[cfg(feature = "serde")]
impl<'de, K, V: serde::Deserialize<'de>> serde::Deserialize<'de> for PerfectMap<K,V> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de> {
        use std::borrow::Cow;

        #[repr(transparent)]
        struct CowBytes<'de>(Cow<'de, [u8]>);

        impl<'de> serde::Deserialize<'de> for CowBytes<'de> {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de> {
                if deserializer.is_human_readable() {
                    Vec::<u8>::deserialize(deserializer).map(|v| CowBytes(Cow::Owned(v)))
                } else {
                    <&[u8]>::deserialize(deserializer).map(|v| CowBytes(Cow::Borrowed(v)))
                }
            }
        }

        impl<'de> serde::Serialize for CowBytes<'de> {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer {
                self.0.serialize(serializer)
            }
        }
        
        #[derive(serde::Deserialize)]
        #[serde(field_identifier, rename_all = "lowercase")]
        enum Field { Values, Function }

        #[repr(transparent)]
        struct PerfectMapVisitor<K,V> {
            spooky: PhantomData<(K,V)>
        }
        

        impl<'de, K, V: serde::Deserialize<'de>> serde::de::Visitor<'de> for PerfectMapVisitor<K, V> {
            type Value = PerfectMap<K, V>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("struct PerfectMap")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
                where
                    A: serde::de::SeqAccess<'de>, {
                let values: Vec<V> = seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(0, &self))?;
                let function_bytes: &[u8] = seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(1, &self))?;
                
                let function = GOFunction::read(&mut function_bytes.as_ref()).map_err(|_| serde::de::Error::custom("invalid bytes: expected bytes representing a ph::GOFunction"))?;

                Ok(PerfectMap { function, values, spooky: PhantomData })
            }
            
            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
                where
                    A: serde::de::MapAccess<'de>, {
                let mut values: Option<Vec<V>> = None;
                let mut function_bytes: Option<Cow<'de, [u8]>> = None;

                while let Some(key) = map.next_key()? {
                    match key {
                        Field::Function => {
                            if function_bytes.is_some() { return Err(serde::de::Error::duplicate_field("function")) };

                            function_bytes = Some(map.next_value()?);
                        },
                        Field::Values => {
                            if values.is_some() { return Err(serde::de::Error::duplicate_field("function")) };
                            values = Some(map.next_value()?);
                        }
                    }
                }
                
                let function_bytes = function_bytes.ok_or_else(|| serde::de::Error::missing_field("function"))?;
                let values = values.ok_or_else(|| serde::de::Error::missing_field("values"))?;

                let function = GOFunction::read(&mut function_bytes.as_ref()).map_err(|_| serde::de::Error::custom("invalid bytes: expected bytes representing a ph::GOFunction"))?;


                Ok(PerfectMap { function, values, spooky: PhantomData })
            }
        }
        
        const FIELDS: &'static [&'static str] = &["values", "function"];
        deserializer.deserialize_struct("PerfectMap", FIELDS, PerfectMapVisitor { spooky: PhantomData })
    }
}

#[cfg(test)]
mod test {
    #[cfg(feature = "serde")]
    #[test]
    fn test_serde() {
        use crate::PerfectMap;

        let map: PerfectMap<String, i32> = PerfectMap::new(&["a".into(), "b".into(), "c".into(), "d".into()], vec![1,2,3,4]);

        assert_eq!(map.get("a"), Some(&1i32));
        assert_eq!(map.get("b"), Some(&2i32));
        assert_eq!(map.get("c"), Some(&3i32));
        assert_eq!(map.get("d"), Some(&4i32));

        let serialized_map = serde_json::to_string(&map).unwrap();
        let deserialized_map: PerfectMap<String, i32> = serde_json::from_str(&serialized_map).unwrap();

        assert_eq!(deserialized_map.get("a"), Some(&1i32));
        assert_eq!(deserialized_map.get("b"), Some(&2i32));
        assert_eq!(deserialized_map.get("c"), Some(&3i32));
        assert_eq!(deserialized_map.get("d"), Some(&4i32));
    }
}