// Virtual interfaces (IEEE 1800-2017 25.9): handles of interface instances in variables,
// class properties, arguments, queues and type parameters; reads, writes, waits and
// subroutine calls through them reach the instance the handle refers to.
interface bus_if(input logic clk);
  logic [7:0] data;
  logic valid;
  int count;
  task pulse(input logic [7:0] d);
    data = d;
    count++;
  endtask
  function int twice();
    return 2 * data;
  endfunction
endinterface

class Driver;
  virtual bus_if vif;
  function new(virtual bus_if v); vif = v; endfunction
  task drive(logic [7:0] d);
    @(posedge vif.clk);
    vif.data <= d;
    vif.valid = 1;
    @(posedge vif.clk);
    vif.valid = 0;
  endtask
endclass

class Holder #(type T = int);
  static T items[$];
  static function void put(T x); items.push_back(x); endfunction
endclass

module top;
  logic clk = 0;
  always #5 clk = ~clk;
  bus_if b0(clk);
  bus_if b1(clk);
  bus_if arr[2](clk);
  virtual bus_if v, w;
  virtual interface bus_if q[$];
  Driver d;
  int bad = 0;
  initial begin
    if (v != null) bad++;
    v = b0;
    w = top.b1;
    if (v == w || v == null) bad++;
    v.data = 3;
    w.data = 4;
    if (b0.data !== 3 || b1.data !== 4 || v.data !== 3) bad++;
    // the handle chooses the instance each time
    w = v;
    w.data = 9;
    if (b0.data !== 9 || b1.data !== 4) bad++;
    d = new(b1);
    d.drive(8'h5a);
    if (b1.data !== 8'h5a || b1.valid !== 0 || b0.data !== 9) bad++;
    // subroutines of the interface
    v.pulse(8'h21);
    if (b0.data !== 8'h21 || b0.count != 1 || v.twice() != 8'h42) bad++;
    // elements of an instance array; a queue of handles
    q.push_back(arr[1]);
    q.push_back(arr[0]);
    q[0].data = 1;
    q[1].data = 2;
    if (arr[0].data !== 2 || arr[1].data !== 1) bad++;
    // a type parameter
    Holder#(virtual bus_if)::put(b1);
    if (Holder#(virtual bus_if)::items[0].data !== 8'h5a) bad++;
    // waits on a value through a handle
    fork
      #12 b1.valid = 1;
    join_none
    w = b1;
    wait (w.valid);
    if ($time != 27) bad++;
    while (w.valid !== 0) #1 if (w.valid) w.valid = 0;
    if (bad == 0) $display("PASSED");
    else $display("FAILED: %0d", bad);
    $finish;
  end
endmodule
