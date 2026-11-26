// The built-in package `std` (IEEE 1800-2017 9.7, 15.3, 15.4): processes, semaphores and
// mailboxes, parsed before the sources that use them and imported into every compilation
// unit. A blocking method waits on a condition on the object's properties.
package std;
  // A process object stands for one process (a fork branch started again is another);
  // `self` gives the same object each time. The `$__process_*` functions are the
  // simulator's.
  class process;
    typedef enum { FINISHED, RUNNING, WAITING, SUSPENDED, KILLED } state;
    local bit [63:0] id;
    local static process all[bit [63:0]];
    static function process self();
      bit [63:0] i = $__process_self();
      if (!all.exists(i)) begin
        process p = new;
        p.id = i;
        all[i] = p;
      end
      return all[i];
    endfunction
    function state status();
      return state'($__process_status(id));
    endfunction
    function void kill();
      $__process_kill(id);
    endfunction
    task await();
      $__process_await(id);
    endtask
    // the process's random number generator (IEEE 1800-2017 18.13.3-5)
    function void srandom(int seed);
      $__process_srandom(id, seed);
    endfunction
    function string get_randstate();
      return $__process_get_randstate(id);
    endfunction
    function void set_randstate(string state);
      $__process_set_randstate(id, state);
    endfunction
  endclass

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
