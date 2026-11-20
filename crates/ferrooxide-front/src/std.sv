// The built-in package `std` (IEEE 1800-2017 15.3, 15.4): semaphores and mailboxes,
// parsed before the sources that use them and imported into every compilation unit.
// A blocking method waits on a condition on the object's properties.
package std;
  class semaphore;
    local int keys;
    function new(int keyCount = 0);
      keys = keyCount;
    endfunction
    function void put(int keyCount = 1);
      keys += keyCount;
    endfunction
    task get(int keyCount = 1);
      wait (keys >= keyCount);
      keys -= keyCount;
    endtask
    function int try_get(int keyCount = 1);
      if (keys < keyCount) return 0;
      keys -= keyCount;
      return 1;
    endfunction
  endclass

  class mailbox #(type T = int);
    local T items[$];
    local int bound;
    function new(int bound = 0);
      this.bound = bound;
    endfunction
    function int num();
      return items.size();
    endfunction
    task put(T message);
      wait (bound <= 0 || items.size() < bound);
      items.push_back(message);
    endtask
    function int try_put(T message);
      if (bound > 0 && items.size() >= bound) return 0;
      items.push_back(message);
      return 1;
    endfunction
    task get(output T message);
      wait (items.size() > 0);
      message = items.pop_front();
    endtask
    function int try_get(output T message);
      if (items.size() == 0) return 0;
      message = items.pop_front();
      return 1;
    endfunction
    task peek(output T message);
      wait (items.size() > 0);
      message = items[0];
    endtask
    function int try_peek(output T message);
      if (items.size() == 0) return 0;
      message = items[0];
      return 1;
    endfunction
  endclass
endpackage
