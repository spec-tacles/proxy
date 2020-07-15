local bucket_key = KEYS[1]
local bucket_size_key = KEYS[2]
local notify_key = KEYS[3]

local new_bucket_size = tonumber(ARGV[1])
local expires_at = tonumber(ARGV[2])

if new_bucket_size > 0 then
	local was_set = redis.call("SET", bucket_size_key, new_bucket_size, "NX")
	if was_set ~= nil then
		redis.call("INCRBY", bucket_key, new_bucket_size)
	end
end

if expires_at > 0 then
	redis.call("SET", bucket_key, 1, "NX")
	redis.call("PEXPIREAT", bucket_key, expires_at)
end

local ttl = redis.call("TTL", bucket_key)
if ttl < 0 then -- key has no expire or doesn't exist
	redis.call("INCR", bucket_key)
	redis.call("PUBLISH", notify_key, bucket_key)
end
