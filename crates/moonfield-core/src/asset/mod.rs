//! # Asset Management System
//!
//! Centralized asset management system inspired by Bevy's asset architecture.
//! Provides loading, caching, and management of various asset types with handles
//! for safe resource access.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::Path;

use crate::allocator::{Handle, Pool};

pub use common::*;
pub use loader::*;
pub mod common;
pub mod loader;

/// Unique identifier for assets across all asset types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AssetId(u64);

impl AssetId {
    /// Creates a new asset ID from a u64 value.
    pub fn new(id: u64) -> Self {
        AssetId(id)
    }

    /// Returns the internal u64 value of this asset ID.
    pub fn id(&self) -> u64 {
        self.0
    }
}

/// Trait that all assets must implement.
pub trait Asset: Send + Sync + 'static {
    /// Returns the type ID of this asset type.
    fn type_id(&self) -> TypeId {
        TypeId::of::<Self>()
    }
    
    /// Returns a reference to the asset as `Any` for downcasting.
    fn as_any(&self) -> &dyn std::any::Any;
    
    /// Returns a mutable reference to the asset as `Any` for downcasting.
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}

/// Strong handle to an asset that keeps the asset alive.
#[derive(Debug, Clone)]
pub struct AssetHandle<T: Asset> {
    handle: Handle<AssetContainer>,
    _phantom: std::marker::PhantomData<T>,
}

impl<T: Asset> AssetHandle<T> {
    /// Creates a new asset handle from an internal handle.
    pub fn new(handle: Handle<AssetContainer>) -> Self {
        Self {
            handle,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Returns the internal handle.
    pub fn as_handle(&self) -> Handle<AssetContainer> {
        self.handle
    }
}

impl<T: Asset> PartialEq for AssetHandle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.handle == other.handle
    }
}

impl<T: Asset> Eq for AssetHandle<T> {}

impl<T: Asset> Hash for AssetHandle<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.handle.index.hash(state);
        self.handle.generation.hash(state);
    }
}

/// Container for storing assets of any type.
pub struct AssetContainer {
    /// The actual asset data.
    pub asset: Box<dyn Asset>,
    /// Unique ID for this asset.
    pub id: AssetId,
    /// Reference count for keeping track of strong handles.
    pub strong_count: usize,
    /// Whether this asset is loaded and ready for use.
    pub loaded: bool,
}

impl AssetContainer {
    /// Creates a new asset container.
    pub fn new(asset: Box<dyn Asset>, id: AssetId) -> Self {
        Self {
            asset,
            id,
            strong_count: 0,
            loaded: true,
        }
    }
}

/// Trait for loading assets from external sources.
pub trait AssetLoader<T: Asset>: Send + Sync {
    /// Loads an asset from a file path.
    fn load(&self, path: &Path) -> Result<T, Box<dyn std::error::Error>>;
}

/// Storage for assets of a specific type.
pub struct AssetStorage<T: Asset> {
    /// Pool for storing assets of type T.
    pool: Pool<AssetContainer>,
    /// Map from file paths to asset handles for quick lookup.
    path_to_handle: HashMap<String, Handle<AssetContainer>>,
    /// Next available asset ID.
    next_id: u64,
    /// Phantom data to hold the type parameter
    _phantom: std::marker::PhantomData<T>,
}

impl<T: Asset> Default for AssetStorage<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Asset> AssetStorage<T> {
    /// Creates a new asset storage.
    pub fn new() -> Self {
        Self {
            pool: Pool::new(),
            path_to_handle: HashMap::new(),
            next_id: 0,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Adds an asset to the storage and returns a handle to it.
    pub fn add(&mut self, asset: T) -> AssetHandle<T> {
        let id = AssetId::new(self.next_id);
        self.next_id += 1;
        
        let container = AssetContainer::new(Box::new(asset), id);
        let handle = self.pool.spawn(container);
        
        AssetHandle::new(handle)
    }

    /// Gets a reference to an asset from its handle.
    pub fn get(&self, handle: &AssetHandle<T>) -> Option<&T> {
        if let Some(container) = self.pool.get(handle.as_handle()) {
            container.asset.as_any().downcast_ref::<T>()
        } else {
            None
        }
    }

    /// Gets a mutable reference to an asset from its handle.
    pub fn get_mut(&mut self, handle: &AssetHandle<T>) -> Option<&mut T> {
        if let Some(container) = self.pool.get_mut(handle.as_handle()) {
            container.asset.as_any_mut().downcast_mut::<T>()
        } else {
            None
        }
    }

    /// Checks if an asset handle is valid.
    pub fn contains(&self, handle: &AssetHandle<T>) -> bool {
        self.pool.is_valid(handle.as_handle())
    }

    /// Removes an asset from the storage.
    pub fn remove(&mut self, handle: AssetHandle<T>) -> bool {
        self.pool.free(handle.as_handle())
    }
}

/// Server responsible for loading, storing, and managing all assets.
pub struct AssetServer {
    /// Storage for different asset types.
    storages: HashMap<TypeId, Box<dyn Any>>,
    /// Loader for different asset types.
    loaders: HashMap<TypeId, Box<dyn Any>>,
    /// Global asset ID counter.
    global_asset_counter: u64,
}

impl Default for AssetServer {
    fn default() -> Self {
        Self::new()
    }
}

impl AssetServer {
    /// Creates a new asset server.
    pub fn new() -> Self {
        Self {
            storages: HashMap::new(),
            loaders: HashMap::new(),
            global_asset_counter: 0,
        }
    }

    /// Registers a loader for a specific asset type.
    pub fn register_loader<T: Asset + 'static>(&mut self, loader: Box<dyn AssetLoader<T>>) {
        let type_id = TypeId::of::<T>();
        self.loaders.insert(type_id, Box::new(loader));
    }

    /// Loads an asset from a file path.
    pub fn load<T: Asset + 'static>(&mut self, path: &str) -> Result<AssetHandle<T>, Box<dyn std::error::Error>> {
        let type_id = TypeId::of::<T>();
        
        // Check if asset is already loaded
        if let Some(storage_any) = self.storages.get(&type_id) {
            if let Some(storage) = storage_any.downcast_ref::<AssetStorage<T>>() {
                if let Some(existing_handle) = storage.path_to_handle.get(path) {
                    // Return existing handle if found
                    return Ok(AssetHandle::new(*existing_handle));
                }
            }
        }
        
        // Load the asset using the registered loader first
        let asset = if let Some(loader_box) = self.loaders.get(&type_id) {
            let loader = loader_box.downcast_ref::<Box<dyn AssetLoader<T>>>().unwrap();
            loader.load(Path::new(path))?
        } else {
            return Err(format!("No loader registered for asset type: {:?}", type_id).into());
        };
        
        // Now get or create storage and add the asset
        let storage = self.get_or_create_storage::<T>();
        let handle = storage.add(asset);
        
        // Store the path-to-handle mapping
        storage.path_to_handle.insert(path.to_string(), handle.as_handle());
        
        Ok(handle)
    }

    /// Adds an asset directly to the server.
    pub fn add<T: Asset + 'static>(&mut self, asset: T) -> AssetHandle<T> {
        let storage = self.get_or_create_storage::<T>();
        storage.add(asset)
    }

    /// Gets a reference to an asset from its handle.
    pub fn get<T: Asset + 'static>(&self, handle: &AssetHandle<T>) -> Option<&T> {
        if let Some(storage) = self.get_storage::<T>() {
            storage.get(handle)
        } else {
            None
        }
    }

    /// Gets a mutable reference to an asset from its handle.
    pub fn get_mut<T: Asset + 'static>(&mut self, handle: &AssetHandle<T>) -> Option<&mut T> {
        if let Some(storage) = self.get_storage_mut::<T>() {
            storage.get_mut(handle)
        } else {
            None
        }
    }

    /// Checks if an asset handle is valid.
    pub fn contains<T: Asset + 'static>(&self, handle: &AssetHandle<T>) -> bool {
        if let Some(storage) = self.get_storage::<T>() {
            storage.contains(handle)
        } else {
            false
        }
    }

    /// Removes an asset from the server.
    pub fn remove<T: Asset + 'static>(&mut self, handle: AssetHandle<T>) -> bool {
        if let Some(storage) = self.get_storage_mut::<T>() {
            storage.remove(handle)
        } else {
            false
        }
    }

    /// Gets or creates a storage for a specific asset type.
    fn get_or_create_storage<T: Asset + 'static>(&mut self) -> &mut AssetStorage<T> {
        let type_id = TypeId::of::<T>();
        
        if !self.storages.contains_key(&type_id) {
            let storage: AssetStorage<T> = AssetStorage::new();
            self.storages.insert(type_id, Box::new(storage));
        }
        
        self.storages
            .get_mut(&type_id)
            .unwrap()
            .downcast_mut::<AssetStorage<T>>()
            .unwrap()
    }

    /// Gets a storage for a specific asset type.
    fn get_storage<T: Asset + 'static>(&self) -> Option<&AssetStorage<T>> {
        let type_id = TypeId::of::<T>();
        
        self.storages.get(&type_id)?.downcast_ref::<AssetStorage<T>>()
    }

    /// Gets a mutable storage for a specific asset type.
    fn get_storage_mut<T: Asset + 'static>(&mut self) -> Option<&mut AssetStorage<T>> {
        let type_id = TypeId::of::<T>();
        
        self.storages.get_mut(&type_id)?.downcast_mut::<AssetStorage<T>>()
    }
}