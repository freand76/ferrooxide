// Events as class properties (IEEE 1800-2017 15.5, 15.5.5): each object's event is its
// own, triggers wake the processes waiting for it (also inside methods and together with
// a module's signal), `.triggered`, assignments make two names refer to the same event,
// a null event, `wait` for `.triggered`, arrays of events and events of a base class.
class B;
  event be;
endclass

class C extends B;
  event e, f[3];
  int n;
  task wait_e();
    @e;
    n++;
  endtask
  function void fire();
    ->e;
  endfunction
endclass

module t;
  integer errors = 0;
  C a, b, q[$];
  reg clk = 0;
  int got, order[$];
  bit trig;
  time t0;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0d want %0d", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    a = new;
    b = new;
    q.push_back(a);
    fork
      a.wait_e();
      begin @(a.e); got = got + 1; end
      begin @(b.e); got = got + 100; end
      begin @(q[0].f[1]); order.push_back(1); end
      begin @(a.be); order.push_back(2); end
      begin @(a.f[2] or posedge clk); order.push_back(3); end
    join_none
    #1;
    check(`__LINE__, a.e.triggered, 0);
    a.fire();
    check(`__LINE__, a.e.triggered, 1);
    check(`__LINE__, b.e.triggered, 0);
    #1;
    check(`__LINE__, a.n, 1);
    check(`__LINE__, got, 1);
    check(`__LINE__, a.e.triggered, 0);
    // only the element that was triggered
    ->a.f[1];
    ->a.be;
    #1;
    check(`__LINE__, order.size(), 2);
    // the module's signal ends the wait as well
    clk = 1;
    #1;
    check(`__LINE__, order.size(), 3);
    check(`__LINE__, order[0] * 100 + order[1] * 10 + order[2], 123);
    // the later trigger of the element no longer wakes it
    ->a.f[2];
    #1;
    check(`__LINE__, order.size(), 3);
    // b's event is another one: still waiting
    check(`__LINE__, got, 1);
    // an assignment: b.e is now a.e's event
    b.e = a.e;
    fork
      begin @(b.e); got = got + 10; end
    join_none
    #1;
    ->a.e;
    #1;
    check(`__LINE__, got, 11);
    // a null event is never triggered
    b.e = null;
    ->b.e;
    trig = b.e.triggered;
    check(`__LINE__, trig, 0);
    // a trigger from a woken process, seen later in the same time step
    fork
      begin @(a.e); ->b.be; end
      begin #1 ->a.e; end
    join_none
    #1;
    #0;
    check(`__LINE__, b.be.triggered, 1);
    // `wait (e.triggered)`: woken by the trigger, or at once after one in this time step
    t0 = $time;
    fork
      begin wait (a.f[0].triggered); got = got + 1000; end
      #3 ->a.f[0];
    join
    check(`__LINE__, $time - t0, 3);
    wait (a.f[0].triggered);
    check(`__LINE__, got, 1011);
    if (errors == 0) $display("PASSED");
  end
endmodule
