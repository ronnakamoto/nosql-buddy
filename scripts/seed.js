// E-commerce seed data for NoSQLBuddy demo.
// Creates: shopkeeper database with products, customers, orders, and inventory logs.
// Run via: docker compose run --rm seeder
// Or:      docker exec mongo-rs mongosh /scripts/seed.js

const db = db.getSiblingDB("shopkeeper");

// ─── Clean slate ──────────────────────────────────────────────────────
db.products.deleteMany({});
db.customers.deleteMany({});
db.orders.deleteMany({});
db.inventory_log.deleteMany({});
db.categories.deleteMany({});

// ─── Categories ───────────────────────────────────────────────────────
const categories = [
  { name: "Electronics", slug: "electronics", description: "Gadgets, devices, and accessories" },
  { name: "Audio", slug: "audio", description: "Headphones, speakers, and microphones" },
  { name: "Wearables", slug: "wearables", description: "Smartwatches and fitness trackers" },
  { name: "Home", slug: "home", description: "Smart home devices and appliances" },
  { name: "Accessories", slug: "accessories", description: "Cables, chargers, and cases" },
];
const categoryIds = {};
categories.forEach((cat) => {
  const res = db.categories.insertOne({
    ...cat,
    created_at: new Date(),
  });
  categoryIds[cat.slug] = res.insertedId;
});

// ─── Products ─────────────────────────────────────────────────────────
const products = [
  {
    sku: "WH-1000XM5",
    name: "Sony WH-1000XM5 Wireless Headphones",
    category: "audio",
    price: 399.99,
    cost: 280.00,
    stock: 145,
    brand: "Sony",
    rating: 4.7,
    tags: ["noise-cancelling", "bluetooth", "over-ear"],
    specs: { battery_life_hrs: 30, weight_g: 250, connectivity: "Bluetooth 5.2" },
    created_at: new Date("2025-01-15"),
  },
  {
    sku: "APP-AIRPODS-PRO2",
    name: "Apple AirPods Pro (2nd Gen)",
    category: "audio",
    price: 249.00,
    cost: 170.00,
    stock: 320,
    brand: "Apple",
    rating: 4.6,
    tags: ["noise-cancelling", "bluetooth", "in-ear"],
    specs: { battery_life_hrs: 6, weight_g: 5.3, connectivity: "Bluetooth 5.3" },
    created_at: new Date("2025-01-20"),
  },
  {
    sku: "SAMS-BUDS3-PRO",
    name: "Samsung Galaxy Buds3 Pro",
    category: "audio",
    price: 199.99,
    cost: 130.00,
    stock: 89,
    brand: "Samsung",
    rating: 4.3,
    tags: ["noise-cancelling", "bluetooth", "in-ear"],
    specs: { battery_life_hrs: 6, weight_g: 5.4, connectivity: "Bluetooth 5.3" },
    created_at: new Date("2025-02-01"),
  },
  {
    sku: "AW-S9-45MM",
    name: "Apple Watch Series 9 45mm",
    category: "wearables",
    price: 429.00,
    cost: 300.00,
    stock: 67,
    brand: "Apple",
    rating: 4.8,
    tags: ["smartwatch", "fitness", "gps"],
    specs: { display: "45mm LT3 OLED", battery_life_hrs: 18, water_resistance: "50m" },
    created_at: new Date("2025-01-10"),
  },
  {
    sku: "GARMIN-F7-SS",
    name: "Garmin Fenix 7 Sapphire Solar",
    category: "wearables",
    price: 699.99,
    cost: 480.00,
    stock: 34,
    brand: "Garmin",
    rating: 4.9,
    tags: ["smartwatch", "fitness", "gps", "solar"],
    specs: { display: "1.3in MIP", battery_life_hrs: 288, water_resistance: "100m" },
    created_at: new Date("2025-01-05"),
  },
  {
    sku: "HOME-NEST-HUB",
    name: "Google Nest Hub 2nd Gen",
    category: "home",
    price: 99.99,
    cost: 65.00,
    stock: 210,
    brand: "Google",
    rating: 4.4,
    tags: ["smart-home", "display", "voice-assistant"],
    specs: { display: "7in 1024x600", speakers: "2.0", connectivity: "Wi-Fi 5, Bluetooth 5.0" },
    created_at: new Date("2025-02-10"),
  },
  {
    sku: "HOME-ECHO-DOT5",
    name: "Amazon Echo Dot 5th Gen",
    category: "home",
    price: 49.99,
    cost: 32.00,
    stock: 450,
    brand: "Amazon",
    rating: 4.5,
    tags: ["smart-home", "speaker", "voice-assistant"],
    specs: { speakers: "1.6in", connectivity: "Wi-Fi 6, Bluetooth 5.2" },
    created_at: new Date("2025-02-15"),
  },
  {
    sku: "ACC-USBC-100W",
    name: "USB-C 100W GaN Charger",
    category: "accessories",
    price: 39.99,
    cost: 22.00,
    stock: 580,
    brand: "Anker",
    rating: 4.7,
    tags: ["charger", "usb-c", "gan"],
    specs: { ports: 3, power_w: 100, weight_g: 120 },
    created_at: new Date("2025-03-01"),
  },
  {
    sku: "ACC-TB4-CABLE-1M",
    name: "Thunderbolt 4 Cable 1m",
    category: "accessories",
    price: 29.99,
    cost: 18.00,
    stock: 340,
    brand: "CalDigit",
    rating: 4.6,
    tags: ["cable", "thunderbolt", "usb-c"],
    specs: { length_m: 1, bandwidth_gbps: 40, power_w: 100 },
    created_at: new Date("2025-03-05"),
  },
  {
    sku: "ELEC-MBP-14-M3",
    name: "MacBook Pro 14in M3 8GB/512GB",
    category: "electronics",
    price: 1599.00,
    cost: 1200.00,
    stock: 23,
    brand: "Apple",
    rating: 4.9,
    tags: ["laptop", "apple-silicon", "retina"],
    specs: { cpu: "M3", ram_gb: 8, storage_gb: 512, display: "14.2in Liquid Retina XDR" },
    created_at: new Date("2025-01-01"),
  },
  {
    sku: "ELEC-IP15-128",
    name: "iPhone 15 128GB",
    category: "electronics",
    price: 799.00,
    cost: 560.00,
    stock: 178,
    brand: "Apple",
    rating: 4.7,
    tags: ["smartphone", "5g", "usb-c"],
    specs: { display: "6.1in OLED", storage_gb: 128, chipset: "A16 Bionic" },
    created_at: new Date("2025-01-08"),
  },
  {
    sku: "ELEC-PIX8-128",
    name: "Google Pixel 8 128GB",
    category: "electronics",
    price: 699.00,
    cost: 490.00,
    stock: 92,
    brand: "Google",
    rating: 4.5,
    tags: ["smartphone", "5g", "android"],
    specs: { display: "6.2in OLED", storage_gb: 128, chipset: "Tensor G3" },
    created_at: new Date("2025-01-12"),
  },
];

const productIds = {};
products.forEach((p) => {
  p.category_id = categoryIds[p.category];
  delete p.category;
  const res = db.products.insertOne(p);
  productIds[p.sku] = res.insertedId;
});

// ─── Customers ────────────────────────────────────────────────────────
const customers = [
  {
    email: "alice.chen@example.com",
    name: { first: "Alice", last: "Chen" },
    phone: "+1-415-555-0101",
    address: { street: "1234 Market St", city: "San Francisco", state: "CA", zip: "94103", country: "USA" },
    created_at: new Date("2025-01-15"),
    total_orders: 0,
    total_spent: 0,
  },
  {
    email: "bob.martinez@example.com",
    name: { first: "Bob", last: "Martinez" },
    phone: "+1-512-555-0202",
    address: { street: "456 Congress Ave", city: "Austin", state: "TX", zip: "78701", country: "USA" },
    created_at: new Date("2025-02-01"),
    total_orders: 0,
    total_spent: 0,
  },
  {
    email: "carol.singh@example.com",
    name: { first: "Carol", last: "Singh" },
    phone: "+44-20-7946-0303",
    address: { street: "221B Baker St", city: "London", state: "", zip: "NW1 6XE", country: "UK" },
    created_at: new Date("2025-02-20"),
    total_orders: 0,
    total_spent: 0,
  },
  {
    email: "david.kim@example.com",
    name: { first: "David", last: "Kim" },
    phone: "+82-2-555-0404",
    address: { street: "89 Gangnam-daero", city: "Seoul", state: "", zip: "06232", country: "Korea" },
    created_at: new Date("2025-03-01"),
    total_orders: 0,
    total_spent: 0,
  },
  {
    email: "eva.novak@example.com",
    name: { first: "Eva", last: "Novak" },
    phone: "+1-206-555-0505",
    address: { street: "500 Pine St", city: "Seattle", state: "WA", zip: "98101", country: "USA" },
    created_at: new Date("2025-03-10"),
    total_orders: 0,
    total_spent: 0,
  },
];

const customerIds = {};
customers.forEach((c) => {
  const res = db.customers.insertOne(c);
  customerIds[c.email] = res.insertedId;
});

// ─── Orders ───────────────────────────────────────────────────────────
const orders = [
  {
    order_number: "ORD-2025-0001",
    customer_id: customerIds["alice.chen@example.com"],
    customer_email: "alice.chen@example.com",
    status: "delivered",
    items: [
      { sku: "WH-1000XM5", name: "Sony WH-1000XM5", qty: 1, price: 399.99 },
      { sku: "ACC-USBC-100W", name: "USB-C 100W GaN Charger", qty: 1, price: 39.99 },
    ],
    subtotal: 439.98,
    shipping: 0,
    tax: 35.20,
    total: 475.18,
    shipping_address: { street: "1234 Market St", city: "San Francisco", state: "CA", zip: "94103", country: "USA" },
    placed_at: new Date("2025-03-15T10:30:00Z"),
    delivered_at: new Date("2025-03-18T14:00:00Z"),
  },
  {
    order_number: "ORD-2025-0002",
    customer_id: customerIds["bob.martinez@example.com"],
    customer_email: "bob.martinez@example.com",
    status: "shipped",
    items: [
      { sku: "AW-S9-45MM", name: "Apple Watch Series 9 45mm", qty: 1, price: 429.00 },
    ],
    subtotal: 429.00,
    shipping: 0,
    tax: 34.32,
    total: 463.32,
    shipping_address: { street: "456 Congress Ave", city: "Austin", state: "TX", zip: "78701", country: "USA" },
    placed_at: new Date("2025-04-02T09:15:00Z"),
    shipped_at: new Date("2025-04-03T16:00:00Z"),
  },
  {
    order_number: "ORD-2025-0003",
    customer_id: customerIds["carol.singh@example.com"],
    customer_email: "carol.singh@example.com",
    status: "processing",
    items: [
      { sku: "ELEC-MBP-14-M3", name: "MacBook Pro 14in M3", qty: 1, price: 1599.00 },
      { sku: "ACC-TB4-CABLE-1M", name: "Thunderbolt 4 Cable 1m", qty: 2, price: 29.99 },
      { sku: "HOME-NEST-HUB", name: "Google Nest Hub 2nd Gen", qty: 1, price: 99.99 },
    ],
    subtotal: 1658.97,
    shipping: 25.00,
    tax: 132.72,
    total: 1816.69,
    shipping_address: { street: "221B Baker St", city: "London", state: "", zip: "NW1 6XE", country: "UK" },
    placed_at: new Date("2025-04-10T11:45:00Z"),
  },
  {
    order_number: "ORD-2025-0004",
    customer_id: customerIds["david.kim@example.com"],
    customer_email: "david.kim@example.com",
    status: "delivered",
    items: [
      { sku: "ELEC-PIX8-128", name: "Google Pixel 8 128GB", qty: 1, price: 699.00 },
      { sku: "SAMS-BUDS3-PRO", name: "Samsung Galaxy Buds3 Pro", qty: 1, price: 199.99 },
    ],
    subtotal: 898.99,
    shipping: 15.00,
    tax: 71.92,
    total: 985.91,
    shipping_address: { street: "89 Gangnam-daero", city: "Seoul", state: "", zip: "06232", country: "Korea" },
    placed_at: new Date("2025-04-05T08:00:00Z"),
    delivered_at: new Date("2025-04-09T10:00:00Z"),
  },
  {
    order_number: "ORD-2025-0005",
    customer_id: customerIds["eva.novak@example.com"],
    customer_email: "eva.novak@example.com",
    status: "pending",
    items: [
      { sku: "GARMIN-F7-SS", name: "Garmin Fenix 7 Sapphire Solar", qty: 1, price: 699.99 },
      { sku: "HOME-ECHO-DOT5", name: "Amazon Echo Dot 5th Gen", qty: 2, price: 49.99 },
    ],
    subtotal: 799.97,
    shipping: 0,
    tax: 64.00,
    total: 863.97,
    shipping_address: { street: "500 Pine St", city: "Seattle", state: "WA", zip: "98101", country: "USA" },
    placed_at: new Date("2025-04-12T15:20:00Z"),
  },
  {
    order_number: "ORD-2025-0006",
    customer_id: customerIds["alice.chen@example.com"],
    customer_email: "alice.chen@example.com",
    status: "cancelled",
    items: [
      { sku: "APP-AIRPODS-PRO2", name: "Apple AirPods Pro (2nd Gen)", qty: 2, price: 249.00 },
    ],
    subtotal: 498.00,
    shipping: 0,
    tax: 39.84,
    total: 537.84,
    shipping_address: { street: "1234 Market St", city: "San Francisco", state: "CA", zip: "94103", country: "USA" },
    placed_at: new Date("2025-04-08T13:00:00Z"),
    cancelled_at: new Date("2025-04-08T15:30:00Z"),
    cancel_reason: "Customer changed mind",
  },
];

orders.forEach((o) => {
  db.orders.insertOne(o);

  // Update customer totals (skip cancelled orders)
  if (o.status !== "cancelled") {
    db.customers.updateOne(
      { _id: o.customer_id },
      { $inc: { total_orders: 1, total_spent: o.total } }
    );
  }

  // Log inventory changes
  o.items.forEach((item) => {
    db.inventory_log.insertOne({
      sku: item.sku,
      order_number: o.order_number,
      change: -item.qty,
      reason: "order",
      timestamp: o.placed_at,
    });
    db.products.updateOne(
      { sku: item.sku },
      { $inc: { stock: -item.qty } }
    );
  });
});

// ─── Restock events ───────────────────────────────────────────────────
const restocks = [
  { sku: "WH-1000XM5", qty: 50, reason: "supplier_restock", date: new Date("2025-03-01") },
  { sku: "APP-AIRPODS-PRO2", qty: 100, reason: "supplier_restock", date: new Date("2025-03-05") },
  { sku: "ELEC-IP15-128", qty: 80, reason: "supplier_restock", date: new Date("2025-03-20") },
  { sku: "ACC-USBC-100W", qty: 200, reason: "supplier_restock", date: new Date("2025-04-01") },
  { sku: "HOME-ECHO-DOT5", qty: 150, reason: "supplier_restock", date: new Date("2025-04-10") },
];

restocks.forEach((r) => {
  db.inventory_log.insertOne({
    sku: r.sku,
    change: r.qty,
    reason: r.reason,
    timestamp: r.date,
  });
  db.products.updateOne(
    { sku: r.sku },
    { $inc: { stock: r.qty } }
  );
});

// ─── Indexes ──────────────────────────────────────────────────────────
db.products.createIndex({ sku: 1 }, { unique: true });
db.products.createIndex({ name: "text" });
db.products.createIndex({ category_id: 1, price: 1 });
db.customers.createIndex({ email: 1 }, { unique: true });
db.orders.createIndex({ order_number: 1 }, { unique: true });
db.orders.createIndex({ customer_id: 1, placed_at: -1 });
db.orders.createIndex({ status: 1 });
db.inventory_log.createIndex({ sku: 1, timestamp: -1 });

// ─── Summary ──────────────────────────────────────────────────────────
print("========================================");
print("  shopkeeper database seeded");
print("========================================");
print(`  categories:    ${db.categories.countDocuments()}`);
print(`  products:      ${db.products.countDocuments()}`);
print(`  customers:     ${db.customers.countDocuments()}`);
print(`  orders:        ${db.orders.countDocuments()}`);
print(`  inventory_log: ${db.inventory_log.countDocuments()}`);
print("");
print("  Databases: shopkeeper");
print("  Collections: categories, products, customers, orders, inventory_log");
print("========================================");
