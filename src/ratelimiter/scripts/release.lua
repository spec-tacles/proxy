local bucket_key = KEYS[1]
local bucket_size_key = KEYS[2]
local notify_key = KEYS[3]

local new_bucket_size = tonumber(ARGV[1])
local expires_at = tonumber(ARGV[2])

if new_bucket_size > 0 then
	local original_bucket_size = tonumber(redis.call("GET", bucket_size_key))
	if original_bucket_size == nil then
		-- if there was no bucket size, we assumed 1
		original_bucket_size = 1
	end

	local diff = new_bucket_size - original_bucket_size
	if diff ~= 0 then
		redis.call("INCRBY", bucket_key, diff)
	end

	redis.call("SET", bucket_size_key, new_bucket_size)
end

redis.call("INCR", bucket_key)

if expires_at > 0 then
	redis.call("PEXPIREAT", bucket_key, expires_at)
end

local ttl = redis.call("TTL", bucket_key)
if ttl < 0 then -- key has no expire or doesn't exist
	redis.call("PUBLISH", notify_key, bucket_key)
end
