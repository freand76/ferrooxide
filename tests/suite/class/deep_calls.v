// Calls nested deeper than the compiler inlines go through subroutines of the process:
// static variables keep their values between calls (IEEE 1800-2017 13.3.1), automatic
// ones are per call (also across recursion), output and array arguments, string results,
// methods and constructors deep in a call chain, tasks that wait, and the same deep
// function called from several processes and from static initializers.
module t;
  integer errors = 0;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0d want %0d", line, got, want);
      errors = errors + 1;
    end
  endtask

  function int counter();
    int n;  // static
    n = n + 1;
    return n;
  endfunction
  function automatic int fact(int k);
    return k <= 1 ? 1 : k * fact(k - 1);
  endfunction
  function automatic void split(input int v, output int hi, output int lo);
    hi = v / 100;
    lo = v % 100;
  endfunction
  function automatic int sum3(int a[3]);
    return a[0] + a[1] + a[2];
  endfunction
  function automatic string tag(string s, int k);
    return $sformatf("%s%0d", s, k);
  endfunction
  task automatic later(input int d, output int t);
    #d t = $time;
  endtask

  class Node;
    int v;
    Node next;
    function new(int x);
      v = x;
    endfunction
    function int total();
      return next == null ? v : v + next.total();
    endfunction
  endclass

  // a chain six calls deep around each of the above
  function automatic int f6(int k); return f5(k); endfunction
  function automatic int f5(int k); return f4(k); endfunction
  function automatic int f4(int k); return f3(k); endfunction
  function automatic int f3(int k); return f2(k); endfunction
  function automatic int f2(int k); return f1(k); endfunction
  function automatic int f1(int k);
    int hi, lo;
    int a[3];
    Node n;
    case (k)
      0: return counter();
      1: return fact(6);
      2: begin split(1234, hi, lo); return hi * 1000 + lo; end
      3: begin a = '{1, 2, 3}; return sum3(a); end
      4: return tag("x", 7).len();
      5: begin
        n = new(1);
        n.next = new(2);
        n.next.next = new(3);
        return n.total();
      end
    endcase
    return -1;
  endfunction
  task automatic t6(input int d, output int r); t5(d, r); endtask
  task automatic t5(input int d, output int r); t4(d, r); endtask
  task automatic t4(input int d, output int r); t3(d, r); endtask
  task automatic t3(input int d, output int r); t2(d, r); endtask
  task automatic t2(input int d, output int r); later(d, r); endtask

  int s1 = f6(1), s2 = f6(5);
  int r1, r2;

  initial begin
    check(`__LINE__, s1, 720);
    check(`__LINE__, s2, 6);
    check(`__LINE__, f6(0), 1);
    check(`__LINE__, f6(0), 2);
    check(`__LINE__, f6(1), 720);
    check(`__LINE__, f6(2), 12034);
    check(`__LINE__, f6(3), 6);
    check(`__LINE__, f6(4), 2);
    check(`__LINE__, f6(5), 6);
    fork
      t6(5, r1);
      t6(3, r2);
    join
    check(`__LINE__, r1, 5);
    check(`__LINE__, r2, 3);
    check(`__LINE__, f6(0), 3);
    if (errors == 0) $display("PASSED");
  end
endmodule
