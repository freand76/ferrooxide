// expected
// ref and const ref arguments (IEEE 1800-2017 13.5.2): the subroutine changes the caller's
// variable, also an element and a queue; a later port without a direction inherits ref.
module t;
  int g;
  int q[$];
  int arr[3];
  function automatic void inc(ref int x, input int by = 1); x += by; endfunction
  function automatic void swap(ref int a, b); int t = a; a = b; b = t; endfunction
  function automatic int total(const ref int a[$]); int s = 0; foreach (a[i]) s += a[i]; return s; endfunction
  function automatic void push(ref int a[$], input int v); a.push_back(v); endfunction
  task automatic set(ref int x, input int v); x = v; endtask
  initial begin
    int y = 1, z = 9;
    inc(y); inc(y, 5); inc(g, 3);
    swap(y, z);
    push(q, 4); push(q, 6);
    inc(arr[1], 2);
    set(g, 11);
    $display("%0d %0d %0d %0d %0d %0d %0d", y, z, g, total(q), q.size(), arr[1], arr[0]);
  end
endmodule
