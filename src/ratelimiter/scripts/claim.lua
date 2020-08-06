local remaining = tonumber(redis.call("GET", KEYS[1]))

if remaining == nil then
	local bucket_size = tonumber(redis.call("GET", KEYS[2]))

	if bucket_size == nil then
		bucket_size = 1
	end

	remaining = bucket_size
	redis.call("SET", KEYS[1], bucket_size)
end

if remaining <= 0 then
	local ttl = redis.call("PTTL", KEYS[1])
	if ttl == nil then return -1 end
	return ttl
end

redis.call("DECR", KEYS[1])
return 0
