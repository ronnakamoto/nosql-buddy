// Auto-initiate the replica set on first container start.
// Runs via docker-entrypoint-initdb.d on MongoDB first boot.
rs.initiate({ _id: "rs0", members: [{ _id: 0, host: "localhost:27017" }] });
