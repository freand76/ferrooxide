// What an enum variable takes (IEEE 1800-2017 6.19.3): a constant or value of its own enum
// type (variables, array elements, struct members, class properties, function results, task
// arguments, method results, conditionals of them) or an explicit cast.
module t;
  typedef enum logic [1:0] {A, B, C} E;
  typedef struct packed { E e; logic f; } S;
  class K;
    E e;
  endclass
  E e, f, arr[2];
  S s;
  K k;
  int errors = 0;
  function E pick(input E x);
    return x == A ? B : C;
  endfunction
  task put(input E x, output E y);
    y = x;
  endtask
  initial begin
    e = B;
    f = e;
    arr[1] = f;
    s.e = arr[1];
    k = new;
    k.e = s.e;
    e = pick(k.e);
    put(e, f);
    e = f.next();
    f = E'(0);
    arr[0] = e == C ? A : B;
    if (e !== A || f !== A || arr[0] !== B || s.e !== B) begin
      $display("FAILED: %0d %0d %0d %0d", e, f, arr[0], s.e);
      errors++;
    end
    if (errors == 0) $display("PASSED");
  end
endmodule
